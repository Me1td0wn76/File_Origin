//! `fo` コマンド。
//!
//! CLI は薄い層に徹する。引数を解釈してコアのユースケースを呼ぶだけで、
//! ロジックはここに置かない（設計方針 P2）。ここにロジックが溜まると
//! CLI と GUI で挙動が食い違う。

mod doctor;
mod scan;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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

    /// ファイルの来歴を表示する
    Show {
        path: PathBuf,
    },

    /// 記録の統計を表示する
    Stats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let platform = fo_platform::current();

    // doctor だけは DB を開かずに動く。
    // 「DB すら開けない」環境の診断に使いたいため。
    if matches!(cli.command, Command::Doctor) {
        doctor::run(platform.as_ref());
        return Ok(());
    }

    let db_path = cli.db.unwrap_or_else(|| platform.paths().database_path());
    let store = Store::open(&db_path)
        .with_context(|| format!("DB を開けません: {}", db_path.display()))?;

    match cli.command {
        Command::Doctor => unreachable!("上で処理済み"),

        Command::Scan { path, hash, no_recursive } => {
            let roots = match path {
                Some(p) => vec![p],
                None => platform.paths().default_download_dirs(),
            };
            if roots.is_empty() {
                anyhow::bail!(
                    "走査対象が見つかりません。パスを明示してください: fo scan <path>"
                );
            }
            for root in roots {
                scan::run(platform.as_ref(), &store, &root, hash, !no_recursive)?;
            }
        }

        Command::Show { path } => show(platform.as_ref(), &store, &path)?,

        Command::Stats => {
            println!("記録済みファイル : {}", store.count_files()?);
            println!("DB               : {}", db_path.display());
        }
    }

    Ok(())
}

fn show(platform: &dyn fo_platform::Platform, store: &Store, path: &std::path::Path) -> Result<()> {
    // scan と同じ正規化を通してから引く。ここがずれると同じファイルが見つからない。
    let abs = platform
        .paths()
        .canonical(path)
        .unwrap_or_else(|_| path.to_path_buf());

    let Some(record) = store.find_by_path(&abs)? else {
        println!("記録がありません: {}", abs.display());
        println!("`fo scan` で取り込んでください。");
        return Ok(());
    };

    println!("パス       : {}", record.current_path.display());
    println!("サイズ     : {} バイト", record.size);
    println!(
        "SHA-256    : {}",
        record
            .sha256
            .as_ref()
            .map(|d| d.as_str())
            .unwrap_or("(未計算)")
    );
    println!("識別子     : {}", record.stable_id);

    let origins = store.origins_of(record.id)?;
    if origins.is_empty() {
        println!("\n入手元     : 記録なし");
        return Ok(());
    }

    println!("\n入手元 ({} 件、確度の高い順):", origins.len());
    for o in &origins {
        println!(
            "  [{}] {}",
            o.confidence.as_str(),
            o.url.as_deref().unwrap_or("(URL なし)")
        );
        if let Some(r) = &o.referrer_url {
            println!("      参照元: {r}");
        }
        println!("      経路  : {}", o.source.as_str());
    }

    Ok(())
}
