//! `fo scan` — 既存ファイルを取り込む。
//!
//! OS が残したメタデータ（Windows の Zone.Identifier / Linux の xattr）を読み、
//! **拡張機能を入れる前にダウンロードしたファイルも救済する**。
//!
//! ハッシュは既定で計算しない（ADR-0007）。安定識別子だけで追跡は始められるので、
//! 数 GB のファイルで初回スキャンを待たせる理由がない。

use std::path::Path;

use anyhow::{Context, Result};
use fo_core::model::Origin;
use fo_platform::Platform;
use fo_store::Store;

/// 走査の深さ上限。シンボリックリンクの輪や、意図せず `/` を走査してしまう
/// 事故で止まらなくなるのを防ぐ。
const MAX_DEPTH: usize = 32;

#[derive(Default)]
struct Summary {
    seen: usize,
    added: usize,
    already_known: usize,
    origins_found: usize,
    errors: usize,
}

pub fn run(
    platform: &dyn Platform,
    store: &Store,
    root: &Path,
    hash: bool,
    recursive: bool,
) -> Result<()> {
    let root = std::fs::canonicalize(root)
        .with_context(|| format!("走査できません: {}", root.display()))?;

    println!("走査中: {}", root.display());

    let mut summary = Summary::default();
    walk(platform, store, &root, recursive, 0, &mut summary, hash);

    println!();
    println!("  検出      : {} ファイル", summary.seen);
    println!("  新規記録  : {}", summary.added);
    println!("  記録済み  : {}", summary.already_known);
    println!("  入手元取得: {}", summary.origins_found);
    if summary.errors > 0 {
        println!("  エラー    : {}（権限不足など。処理は継続しました）", summary.errors);
    }

    if summary.origins_found == 0 && summary.seen > 0 {
        println!();
        println!("入手元が 1 件も取れませんでした。");
        println!("`fo doctor` でこの環境の取得経路を確認してください。");
    }

    Ok(())
}

fn walk(
    platform: &dyn Platform,
    store: &Store,
    dir: &Path,
    recursive: bool,
    depth: usize,
    summary: &mut Summary,
    hash: bool,
) {
    if depth > MAX_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            // 読めないディレクトリで走査全体を止めない。
            // ダウンロードフォルダの一部が読めないのは普通にある。
            summary.errors += 1;
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // シンボリックリンクは辿らない。リンク先が走査範囲の外にあったり、
        // 輪を作っていたりするため。実体は本来の場所で拾う。
        let Ok(meta) = entry.metadata() else {
            summary.errors += 1;
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }

        if meta.is_dir() {
            if recursive {
                walk(platform, store, &path, recursive, depth + 1, summary, hash);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }

        summary.seen += 1;
        if let Err(_e) = record_one(platform, store, &path, &meta, hash, summary) {
            summary.errors += 1;
        }
    }
}

fn record_one(
    platform: &dyn Platform,
    store: &Store,
    path: &Path,
    meta: &std::fs::Metadata,
    hash: bool,
    summary: &mut Summary,
) -> Result<()> {
    let stable_id = platform.identity().stable_id(path)?;

    // 既に記録済みなら、パスだけ更新する。
    // 前回と違う場所にあれば、それが移動の記録になる。
    if let Some(existing) = store.find_by_stable_id(&stable_id)? {
        if existing.current_path != path {
            store.record_path(existing.id, path)?;
        }
        summary.already_known += 1;
        return Ok(());
    }

    let digest = if hash {
        Some(fo_core::hash::sha256_file(path)?)
    } else {
        None
    };

    let file_id = store.insert_file(
        &stable_id,
        path,
        meta.len(),
        digest.as_ref(),
        mtime_secs(meta),
    )?;
    summary.added += 1;

    // OS が持っていた入手元を取り込む。
    // 読めないこと自体はエラーにしない。ファイルの記録は成立しているため。
    if let Ok(os_origins) = platform.origin_meta().read_origin(path) {
        for os_origin in os_origins {
            let origin = Origin::from_os(os_origin, Some(mtime_secs(meta)));
            store.add_origin(file_id, &origin)?;
            summary.origins_found += 1;
        }
    }

    Ok(())
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
