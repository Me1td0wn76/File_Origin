//! `fo` コマンド。
//!
//! CLI は薄い層に徹する。引数を解釈して `fo-app` のユースケースを呼ぶだけで、
//! ロジックはここに置かない（設計方針 P2）。ここにロジックが溜まると
//! CLI と GUI で挙動が食い違う。

// `#[macro_use]` はテキスト順にしか効かない。outln! を使う各モジュールより先に置く。
#[macro_use]
mod out;

mod doctor;
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
    },

    /// ファイルの現在地を引く（ファイル名の一部、または SHA-256）
    Where { query: String },

    /// 記録の統計を表示する
    Stats,
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
        } => search::run(
            &store,
            search::Args {
                name,
                url,
                host,
                since,
                until,
                limit,
            },
        )?,

        Command::Where { query } => search::locate(&store, &query)?,

        Command::Stats => {
            outln!("記録済みファイル : {}", store.count_files()?);
            outln!("DB               : {}", db_path.display());
        }
    }

    Ok(())
}
