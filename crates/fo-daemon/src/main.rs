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
//!
//! ## コンソール窓を持たない
//!
//! 常駐プロセスがウィンドウを持つのはおかしいし、誤って閉じると記録が止まる。
//! リリースビルドでは GUI サブシステムにして窓を出さない。
//! そのぶん `eprintln!` の行き先が無くなるので、**ログはファイルに書く**
//! （`logging` モジュール）。`--foreground` を付けると起動元の端末にも出す。

// デバッグビルドでは今までどおりコンソールに出す。開発中に窓が無いのは不便。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logging;

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

use crate::logging::Logger;

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

    /// 起動元の端末にもログを出す（常駐させず手元で動かすとき）
    #[arg(long)]
    foreground: bool,

    /// ログに入手元 URL を丸ごと残す
    ///
    /// 既定はホストまでに削る。URL は閲覧履歴と同等の機微情報なので、
    /// 調査が要るときだけ明示的に有効にする（README §14）。
    #[arg(long)]
    verbose: bool,
}

/// デーモンの共有状態。
struct Daemon {
    platform: Box<dyn Platform>,
    store: Mutex<Store>,
    roots: Mutex<Vec<PathBuf>>,
    started: Instant,
    shutdown: AtomicBool,
    opts: IngestOptions,
    log: Logger,
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
            log_path: Some(self.log.path().display().to_string()),
        })
    }

    fn stopping(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let platform = fo_platform::current();

    // --foreground のときだけ端末に繋ぐ。既定では窓を出さない。
    let to_stderr = cli.foreground && platform.attach_parent_console();
    let log = Logger::open(&platform.paths().log_dir(), to_stderr, cli.verbose);

    let db_path = cli.db.unwrap_or_else(|| platform.paths().database_path());
    let store = Store::open(&db_path)
        .map_err(|e| {
            // DB が開けないのは致命的。ログに残してから終わる。
            // 窓が無い状態で黙って死ぬと、原因を追う手がかりがゼロになる。
            log.log(&format!(
                "[致命] DB を開けません {}: {e}",
                db_path.display()
            ));
            e
        })
        .with_context(|| format!("DB を開けません: {}", db_path.display()))?;

    let raw_roots = if cli.roots.is_empty() {
        platform.paths().default_download_dirs()
    } else {
        cli.roots.clone()
    };
    if raw_roots.is_empty() {
        anyhow::bail!("監視対象がありません。--root で指定してください。");
    }
    // 根を先に正規化する。監視ライブラリは登録した根をそのまま前置して
    // イベントを返すので、根が正規なら子も正規になる。
    let mut roots = Vec::with_capacity(raw_roots.len());
    for r in raw_roots {
        match platform.paths().canonical(&r) {
            Ok(c) => roots.push(c),
            Err(e) => log.log(&format!(
                "[警告] 監視対象を解決できません {}: {e}",
                r.display()
            )),
        }
    }
    if roots.is_empty() {
        anyhow::bail!("監視対象をひとつも解決できませんでした。");
    }

    // 先に IPC を張る。既に動いているデーモンがあればここで失敗し、
    // 二重起動で DB を取り合うのを防げる。
    let listener = platform
        .ipc()
        .bind()
        .map_err(|e| {
            log.log(&format!(
                "[致命] IPC を開けません（既にデーモンが動いていませんか）: {} — {e}",
                platform.ipc().endpoint_display()
            ));
            e
        })
        .with_context(|| {
            format!(
                "IPC を開けません（既にデーモンが動いていませんか）: {}",
                platform.ipc().endpoint_display()
            )
        })?;

    log.log(&format!(
        "起動 file-origin daemon {}",
        env!("CARGO_PKG_VERSION")
    ));
    log.log(&format!("  DB   : {}", db_path.display()));
    log.log(&format!("  IPC  : {}", platform.ipc().endpoint_display()));
    log.log(&format!("  ログ : {}", log.path().display()));
    for r in &roots {
        log.log(&format!("  監視 : {}", r.display()));
    }

    let daemon = Arc::new(Daemon {
        platform,
        store: Mutex::new(store),
        roots: Mutex::new(roots),
        started: Instant::now(),
        shutdown: AtomicBool::new(false),
        opts: IngestOptions { hash: cli.hash },
        log,
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
    daemon.log.log("停止しました。");
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
            Ok(_) => d.log.log(&format!(
                "  起動時スキャン {}: 新規 {new} / 移動 {moved}",
                root.display()
            )),
            Err(e) => d.log.log(&format!(
                "[警告] 起動時スキャン {} を飛ばしました: {e}",
                root.display()
            )),
        }
    }
    Ok(())
}

fn watch_loop(d: &Daemon) {
    let mut watcher = match d.platform.new_watcher() {
        Ok(w) => w,
        Err(e) => {
            d.log.log(&format!("[致命] 監視を開始できません: {e}"));
            return;
        }
    };
    {
        let roots = d.roots.lock().expect("roots mutex");
        for root in roots.iter() {
            if let Err(e) = watcher.watch(root, true) {
                d.log
                    .log(&format!("[警告] 監視できません {}: {e}", root.display()));
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
                    d.log.log(&format!(
                        "[{}] {}{}",
                        label(verdict, path_changed),
                        path.display(),
                        if os_origins_recorded > 0 {
                            format!("  (入手元 {os_origins_recorded} 件)")
                        } else {
                            String::new()
                        }
                    ));
                }
                WatchEvent::MarkedMissing { path } => {
                    d.log.log(&format!("[見失い] {}", path.display()));
                }
                WatchEvent::NeedsRescan => {
                    d.log
                        .log("[警告] イベントを取りこぼしました。fo scan で補正してください。");
                }
                WatchEvent::Failed { path, error } => {
                    d.log.log(&format!("[失敗] {}: {error}", path.display()));
                }
            },
        );
        drop(store);
        if let Err(e) = r {
            d.log.log(&format!("[致命] 監視が停止しました: {e}"));
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
                d.log.log(&format!("[警告] 接続を受け付けられません: {e}"));
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
                d.log.log(&format!("[警告] 要求を読めません: {e}"));
                return;
            }
        };

        let shutting_down = matches!(req, Request::Shutdown);
        let res = dispatch(d, req);
        if fo_ipc::write_message(reader.get_mut(), &res).is_err() {
            return;
        }
        if shutting_down {
            // 停止フラグを立てただけでは accept() のブロックが解けない。
            // 自分自身に 1 本つないで目を覚まさせる。
            // （accept をノンブロッキングにしてポーリングする手もあるが、
            //   そのために CPU を回し続けるのは常駐プロセスとして筋が悪い）
            let _ = d.platform.ipc().connect();
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
            d.log.log("停止を要求されました。");
            d.shutdown.store(true, Ordering::Relaxed);
            Response::Ok
        }
    }
}

fn record(d: &Daemon, report: DownloadReport) -> Response {
    let path = fo_app::browser::normalize_reported_path(&report.path);

    // 拡張からの報告が届いているかは、拡張側からは確かめようがない。
    // ここに残しておかないと M4 の切り分けができなくなる。
    // URL は既定で伏せる（--verbose で全体）。
    d.log.log(&format!(
        "[報告] {} ← {}{}",
        path.display(),
        d.log.url(report.url.as_deref()),
        report
            .browser
            .as_deref()
            .map(|b| format!(" ({b})"))
            .unwrap_or_default()
    ));

    if !fo_app::browser::is_acceptable(&path) {
        d.log.log("[拒否] ダウンロード途中のファイル名");
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
        Ok((ingested, _)) => {
            d.log.log(&format!(
                "[記録] id={} {}",
                ingested.file_id,
                label(&ingested.verdict, ingested.path_changed)
            ));
            Response::Recorded {
                file_id: ingested.file_id,
                verdict: label(&ingested.verdict, ingested.path_changed).to_string(),
            }
        }
        Err(e) => {
            d.log.log(&format!("[拒否] {e}"));
            err(e)
        }
    }
}

fn err(e: impl std::fmt::Display) -> Response {
    Response::Error {
        message: e.to_string(),
    }
}
