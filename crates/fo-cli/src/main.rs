//! `fo` コマンド。
//!
//! CLI は薄い層に徹する。引数を解釈して `fo-app` のユースケースを呼ぶだけで、
//! ロジックはここに置かない（設計方針 P2）。ここにロジックが溜まると
//! CLI と GUI で挙動が食い違う。

// `#[macro_use]` はテキスト順にしか効かない。outln! を使う各モジュールより先に置く。
#[macro_use]
mod out;

mod daemon;
mod doctor;
mod host;
mod scan;
mod search;
mod show;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fo_app::{IngestOptions, ScanOptions};
use fo_core::Verdict;
use fo_store::Store;

#[derive(Parser)]
#[command(
    name = "fo",
    about = "ダウンロードしたファイルの入手元・来歴を記録する",
    version
)]
struct Cli {
    /// DB のパス。既定はプラットフォームごとのデータディレクトリ。
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 実行環境の能力を診断する
    Doctor,

    /// ディレクトリを走査して既存ファイルを取り込む
    Scan {
        /// 走査するディレクトリ。省略すると既定のダウンロードフォルダ。
        path: Option<PathBuf>,

        /// SHA-256 を同時に計算する（大きいファイルでは時間がかかる）
        #[arg(long)]
        hash: bool,

        /// サブディレクトリを走査しない（既定は再帰する）
        ///
        /// bool フィールドはフラグとして扱われるため、`--recursive` に
        /// default_value_t = true を付けると無効化できなくなる。否定形で受ける。
        #[arg(long)]
        no_recursive: bool,
    },

    /// 入手元を手で登録する（未記録のファイルは同時に取り込む）
    Add {
        path: PathBuf,

        /// 入手元 URL
        #[arg(long)]
        url: String,

        /// 参照元（ダウンロードリンクがあったページ）の URL
        #[arg(long)]
        referrer: Option<String>,
    },

    /// ファイルの来歴を表示する
    Show { path: PathBuf },

    /// 記録を検索する（条件はすべて AND）
    Search {
        /// ファイル名。`*` `?` が使える。ワイルドカード無しなら部分一致
        #[arg(long)]
        name: Option<String>,

        /// 入手元 URL または参照元 URL に含まれる文字列
        #[arg(long)]
        url: Option<String>,

        /// 入手元のホスト名。サブドメインも当たる（example.com は cdn.example.com に一致）
        #[arg(long)]
        host: Option<String>,

        /// 取得日がこの日以降（YYYY-MM-DD、含む）
        #[arg(long)]
        since: Option<String>,

        /// 取得日がこの日まで（YYYY-MM-DD、含む）
        #[arg(long)]
        until: Option<String>,

        /// 最大件数。0 で無制限
        #[arg(long, default_value_t = 50)]
        limit: usize,

        /// 並べ替えの項目
        /// （first-seen / acquired / name / size / confidence）
        #[arg(long, default_value = "first-seen")]
        sort: String,

        /// 昇順にする。既定は項目ごとの自然な向き
        /// （日時・サイズ・確度は降順、名前は昇順）
        #[arg(long, conflicts_with = "desc")]
        asc: bool,

        /// 降順にする
        #[arg(long)]
        desc: bool,
    },

    /// ファイルの現在地を引く（ファイル名の一部、または SHA-256）
    Where { query: String },

    /// 記録の統計を表示する
    Stats,

    /// 常駐サービスの状態を見る・止める
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// ブラウザ拡張の接続先（Native Messaging ホスト）を登録する
    Host {
        #[command(subcommand)]
        action: HostAction,
    },
}

#[derive(Subcommand)]
enum HostAction {
    /// マニフェストを設置する
    Install {
        /// 対象ブラウザ。省略すると全部（chrome / edge / chromium / firefox）
        #[arg(long = "browser")]
        browsers: Vec<String>,

        /// 接続を許可する拡張 ID。chrome://extensions で確認できる
        #[arg(long = "extension-id", required = true)]
        extension_ids: Vec<String>,

        /// 中継プロセスの場所。省略すると fo と同じディレクトリ
        #[arg(long)]
        exe: Option<PathBuf>,
    },
    /// 設置を取り消す
    Uninstall {
        #[arg(long = "browser")]
        browsers: Vec<String>,
    },
    /// 登録状況を表示する
    Status,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// 稼働状況を表示する
    Status,
    /// 生存確認
    Ping,
    /// 停止を要求する
    Stop,
    /// 生の JSON メッセージを 1 件送る（拡張やホストの動作確認用）
    Send {
        /// 行区切り JSON 1 行。例: {"kind":"ping"}
        json: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // `fo search | head` のように読み手が先に終了しただけ。異常ではない。
        Err(e) if out::is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let platform = fo_platform::current();

    // doctor だけは DB を開かずに動く。
    // 「DB すら開けない」環境の診断に使いたいため。
    if matches!(cli.command, Command::Doctor) {
        doctor::run(platform.as_ref())?;
        return Ok(());
    }

    let db_path = cli.db.unwrap_or_else(|| platform.paths().database_path());
    let store =
        Store::open(&db_path).with_context(|| format!("DB を開けません: {}", db_path.display()))?;

    match cli.command {
        Command::Doctor => unreachable!("上で処理済み"),

        Command::Scan {
            path,
            hash,
            no_recursive,
        } => {
            let roots = match path {
                Some(p) => vec![p],
                None => platform.paths().default_download_dirs(),
            };
            if roots.is_empty() {
                anyhow::bail!("走査対象が見つかりません。パスを明示してください: fo scan <path>");
            }
            let opts = ScanOptions {
                recursive: !no_recursive,
                ingest: IngestOptions { hash },
            };
            for root in roots {
                scan::run(platform.as_ref(), &store, &root, opts)?;
            }
        }

        Command::Add {
            path,
            url,
            referrer,
        } => {
            let (ingested, _) = fo_app::add_manual_origin(
                platform.as_ref(),
                &store,
                &path,
                &url,
                referrer.as_deref(),
            )
            .with_context(|| format!("登録できません: {}", path.display()))?;

            let how = match ingested.verdict {
                Verdict::New => "新規に取り込み、",
                Verdict::Copied { .. } => "コピーとして取り込み、",
                Verdict::Moved { .. } => "移動を記録し、",
                Verdict::Updated { .. } => "内容の更新を記録し、",
                Verdict::Same { .. } => "",
            };
            outln!("{how}入手元を登録しました: {url}");
            show::run(platform.as_ref(), &store, &path)?;
        }

        Command::Show { path } => show::run(platform.as_ref(), &store, &path)?,

        Command::Search {
            name,
            url,
            host,
            since,
            until,
            limit,
            sort,
            asc,
            desc,
        } => search::run(
            &store,
            search::Args {
                name,
                url,
                host,
                since,
                until,
                limit,
                sort,
                direction: match (asc, desc) {
                    (true, _) => Some(false),
                    (_, true) => Some(true),
                    _ => None,
                },
            },
        )?,

        Command::Where { query } => search::locate(&store, &query)?,

        Command::Daemon { action } => match action {
            DaemonAction::Status => daemon::status(platform.as_ref())?,
            DaemonAction::Ping => daemon::ping(platform.as_ref())?,
            DaemonAction::Stop => daemon::stop(platform.as_ref())?,
            DaemonAction::Send { json } => daemon::send_raw(platform.as_ref(), &json)?,
        },

        Command::Host { action } => match action {
            HostAction::Install {
                browsers,
                extension_ids,
                exe,
            } => host::install(platform.as_ref(), &browsers, &extension_ids, exe)?,
            HostAction::Uninstall { browsers } => host::uninstall(platform.as_ref(), &browsers)?,
            HostAction::Status => host::status(platform.as_ref())?,
        },

        Command::Stats => {
            outln!("記録済みファイル : {}", store.count_files()?);
            outln!("DB               : {}", db_path.display());
        }
    }

    Ok(())
}
