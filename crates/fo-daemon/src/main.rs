//! `fo-daemon` — 常駐サービス。
//!
//! 役割は 2 つ:
//!
//! 1. **監視**: 対象ディレクトリの変化を拾って取り込む
//! 2. **IPC サーバ**: CLI / GUI / Native Messaging ホストの要求に応える
//!
//! **DB への書き込み口はこのプロセスだけ**（README §9.1）。
//! Native Messaging ホストはブラウザが複数プロファイルから同時に spawn するため、
//! そこから直接 SQLite を触ると書き込みが競合する。
//!
//! ## スレッド構成
//!
//! tokio は使わない。仕事は「1 本の監視ループ」と「たまに来る接続」だけで、
//! 非同期ランタイムを入れても速くならず、依存とビルド時間が増える。
//! 素の std スレッドと `Mutex<Store>` で足りる（Decision Log D13）。
//!
//! ```text
//!   main ──┬── watcher thread  : 監視 → ingest
//!          └── ipc thread × N  : accept ごとに 1 本
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use fo_app::{IngestOptions, Pending, WatchEvent};
use fo_ipc::{DaemonStatus, DownloadReport, Request, Response};
use fo_platform::Platform;
use fo_store::Store;

#[derive(Parser)]
#[command(name = "fo-daemon", about = "File Origin の常駐サービス", version)]
struct Cli {
    /// DB のパス。既定はプラットフォームごとのデータディレクトリ。
    #[arg(long)]
    db: Option<PathBuf>,

    /// 監視するディレクトリ。省略すると既定のダウンロードフォルダ。
    #[arg(long = "root")]
    roots: Vec<PathBuf>,

    /// 起動時に監視対象を走査しない（差分の取り込みを省く）
    #[arg(long)]
    no_initial_scan: bool,

    /// 取り込み時に SHA-256 も計算する
    #[arg(long)]
    hash: bool,
}

/// デーモンの共有状態。
struct Daemon {
    platform: Box<dyn Platform>,
    store: Mutex<Store>,
    roots: Mutex<Vec<PathBuf>>,
    started: Instant,
    shutdown: AtomicBool,
    opts: IngestOptions,
}

impl Daemon {
    fn status(&self) -> Result<DaemonStatus> {
        let store = self.store.lock().expect("store mutex");
        Ok(DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            roots: self
                .roots
                .lock()
                .expect("roots mutex")
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            uptime_secs: self.started.elapsed().as_secs(),
            files: store.count_files()?,
            watching: true,
        })
    }

    fn stopping(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let platform = fo_platform::current();

    let db_path = cli.db.unwrap_or_else(|| platform.paths().database_path());
    let store =
        Store::open(&db_path).with_context(|| format!("DB を開けません: {}", db_path.display()))?;

    let roots = if cli.roots.is_empty() {
        platform.paths().default_download_dirs()
    } else {
        cli.roots.clone()
    };
    if roots.is_empty() {
        anyhow::bail!("監視対象がありません。--root で指定してください。");
    }

    // 先に IPC を張る。既に動いているデーモンがあればここで失敗し、
    // 二重起動で DB を取り合うのを防げる。
    let listener = platform.ipc().bind().with_context(|| {
        format!(
            "IPC を開けません（既にデーモンが動いていませんか）: {}",
            platform.ipc().endpoint_display()
        )
    })?;

    eprintln!("file-origin daemon {}", env!("CARGO_PKG_VERSION"));
    eprintln!("  DB   : {}", db_path.display());
    eprintln!("  IPC  : {}", platform.ipc().endpoint_display());
    for r in &roots {
        eprintln!("  監視 : {}", r.display());
    }

    let daemon = Arc::new(Daemon {
        platform,
        store: Mutex::new(store),
        roots: Mutex::new(roots),
        started: Instant::now(),
        shutdown: AtomicBool::new(false),
        opts: IngestOptions { hash: cli.hash },
    });

    // 起動時の差分スキャン。停止中の変更を拾う唯一の手段（ADR-0005）。
    if !cli.no_initial_scan {
        initial_scan(&daemon)?;
    }

    let watcher_handle = {
        let d = Arc::clone(&daemon);
        std::thread::Builder::new()
            .name("fo-watcher".into())
            .spawn(move || watch_loop(&d))?
    };

    serve(&daemon, listener.as_ref());

    daemon.shutdown.store(true, Ordering::Relaxed);
    let _ = watcher_handle.join();
    eprintln!("停止しました。");
    Ok(())
}

fn initial_scan(d: &Daemon) -> Result<()> {
    let roots = d.roots.lock().expect("roots mutex").clone();
    for root in roots {
        let store = d.store.lock().expect("store mutex");
        let mut new = 0usize;
        let mut moved = 0usize;
        let opts = fo_app::ScanOptions {
            recursive: true,
            ingest: d.opts,
        };
        let res = fo_app::scan_dir(d.platform.as_ref(), &store, &root, opts, &mut |ev| {
            if let fo_app::ScanEvent::File {
                verdict,
                path_changed,
                ..
            } = ev
            {
                match verdict {
                    fo_core::Verdict::New => new += 1,
                    fo_core::Verdict::Moved { .. } => moved += 1,
                    fo_core::Verdict::Same { .. } if path_changed => moved += 1,
                    _ => {}
                }
            }
        });
        match res {
            Ok(_) => eprintln!(
                "  起動時スキャン {}: 新規 {new} / 移動 {moved}",
                root.display()
            ),
            Err(e) => eprintln!("  起動時スキャン {} を飛ばしました: {e}", root.display()),
        }
    }
    Ok(())
}

fn watch_loop(d: &Daemon) {
    let mut watcher = match d.platform.new_watcher() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("監視を開始できません: {e}");
            return;
        }
    };
    {
        let roots = d.roots.lock().expect("roots mutex");
        for root in roots.iter() {
            if let Err(e) = watcher.watch(root, true) {
                eprintln!("監視できません {}: {e}", root.display());
            }
        }
    }

    let mut pending = Pending::default();
    while !d.stopping() {
        let store = d.store.lock().expect("store mutex");
        let r = fo_app::tick(
            d.platform.as_ref(),
            &store,
            watcher.as_mut(),
            &mut pending,
            d.opts,
            &mut |ev| match ev {
                WatchEvent::Ingested {
                    path,
                    verdict,
                    os_origins_recorded,
                    path_changed,
                } => {
                    eprintln!(
                        "[{}] {}{}",
                        label(verdict, path_changed),
                        path.display(),
                        if os_origins_recorded > 0 {
                            format!("  (入手元 {os_origins_recorded} 件)")
                        } else {
                            String::new()
                        }
                    );
                }
                WatchEvent::MarkedMissing { path } => {
                    eprintln!("[見失い] {}", path.display());
                }
                WatchEvent::NeedsRescan => {
                    eprintln!("[警告] イベントを取りこぼしました。fo scan で補正してください。");
                }
                WatchEvent::Failed { path, error } => {
                    eprintln!("[失敗] {}: {error}", path.display());
                }
            },
        );
        drop(store);
        if let Err(e) = r {
            eprintln!("監視が停止しました: {e}");
            return;
        }
    }
}

/// 判定を利用者向けの言葉にする。
///
/// 同一ボリューム内の移動は `Verdict::Same`（識別子が変わらない）だが、
/// 利用者から見れば「移動」。`path_changed` で区別する。
fn label(v: &fo_core::Verdict, path_changed: bool) -> &'static str {
    match v {
        fo_core::Verdict::New => "新規",
        fo_core::Verdict::Same { .. } if path_changed => "移動",
        fo_core::Verdict::Same { .. } => "変更なし",
        fo_core::Verdict::Moved { .. } => "移動",
        fo_core::Verdict::Copied { .. } => "コピー",
        fo_core::Verdict::Updated { .. } => "更新",
    }
}

fn serve(d: &Arc<Daemon>, listener: &dyn fo_platform::IpcListener) {
    while !d.stopping() {
        let stream = match listener.accept() {
            Ok(s) => s,
            Err(e) => {
                if d.stopping() {
                    break;
                }
                eprintln!("接続を受け付けられません: {e}");
                continue;
            }
        };
        let d = Arc::clone(d);
        // 1 接続 1 スレッド。同時接続は CLI・GUI・拡張の数程度なので、
        // スレッドプールを入れるほどではない。
        let _ = std::thread::Builder::new()
            .name("fo-ipc".into())
            .spawn(move || handle(&d, stream));
    }
}

fn handle(d: &Daemon, mut stream: Box<dyn fo_platform::IpcStream>) {
    use std::io::BufReader;

    let mut reader = BufReader::new(&mut stream);
    loop {
        let req: Request = match fo_ipc::read_message(&mut reader) {
            Ok(r) => r,
            // 相手が閉じただけ。異常ではない。
            Err(fo_ipc::Error::Closed) => return,
            Err(e) => {
                eprintln!("要求を読めません: {e}");
                return;
            }
        };

        let res = dispatch(d, req);
        let is_shutdown = matches!(res, Response::Ok) && d.stopping();
        if fo_ipc::write_message(reader.get_mut(), &res).is_err() {
            return;
        }
        if is_shutdown {
            return;
        }
    }
}

fn dispatch(d: &Daemon, req: Request) -> Response {
    match req {
        Request::Ping => Response::Pong,

        Request::Status => match d.status() {
            Ok(s) => Response::Status(s),
            Err(e) => err(e),
        },

        Request::ListRoots => Response::Roots {
            paths: d
                .roots
                .lock()
                .expect("roots mutex")
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        },

        Request::RecordDownload(report) => record(d, report),

        Request::AddRoot { path, recursive } => {
            let path = PathBuf::from(path);
            let store = d.store.lock().expect("store mutex");
            let opts = fo_app::ScanOptions {
                recursive,
                ingest: d.opts,
            };
            match fo_app::scan_dir(d.platform.as_ref(), &store, &path, opts, &mut |_| {}) {
                Ok(canon) => {
                    d.roots.lock().expect("roots mutex").push(canon);
                    // TODO(M3+): 動いている監視器にも watch() を足す。
                    // 現状は再起動で反映される。
                    Response::Ok
                }
                Err(e) => err(e),
            }
        }

        Request::Shutdown => {
            d.shutdown.store(true, Ordering::Relaxed);
            Response::Ok
        }
    }
}

fn record(d: &Daemon, report: DownloadReport) -> Response {
    let path = fo_app::browser::normalize_reported_path(&report.path);
    if !fo_app::browser::is_acceptable(&path) {
        return Response::Error {
            message: format!("ダウンロード途中のファイル名です: {}", path.display()),
        };
    }

    let app_report = fo_app::DownloadReport {
        path,
        url: report.url,
        referrer: report.referrer,
        acquired_at: report.acquired_at,
        browser: report.browser,
        profile: report.profile,
    };

    let store = d.store.lock().expect("store mutex");
    match fo_app::record_download(d.platform.as_ref(), &store, &app_report) {
        Ok((ingested, _)) => Response::Recorded {
            file_id: ingested.file_id,
            verdict: label(&ingested.verdict, ingested.path_changed).to_string(),
        },
        Err(e) => err(e),
    }
}

fn err(e: impl std::fmt::Display) -> Response {
    Response::Error {
        message: e.to_string(),
    }
}
