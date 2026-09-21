//! `fo scan` — 既存ファイルを取り込む。
//!
//! OS が残したメタデータ（Windows の Zone.Identifier / Linux の xattr）を読み、
//! **拡張機能を入れる前にダウンロードしたファイルも救済する**。
//!
//! ハッシュは既定で計算しない（ADR-0007）。安定識別子だけで追跡は始められるので、
//! 数 GB のファイルで初回スキャンを待たせる理由がない。
//!
//! 観測したファイルが既知のどれに対応するかは `fo_core::classify` に委ねる。
//! ここで独自の判定を書くと、テスト済みのはしごと挙動がずれる。

use std::path::Path;

use anyhow::{Context, Result};
use fo_core::model::Origin;
use fo_core::{classify, Observed, Verdict};
use fo_platform::Platform;
use fo_store::Store;

/// 走査の深さ上限。シンボリックリンクの輪や、意図せず `/` を走査してしまう
/// 事故で止まらなくなるのを防ぐ。
const MAX_DEPTH: usize = 32;

#[derive(Default)]
struct Summary {
    seen: usize,
    added: usize,
    unchanged: usize,
    moved: usize,
    copied: usize,
    updated: usize,
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
    let root = platform
        .paths()
        .canonical(root)
        .with_context(|| format!("走査できません: {}", root.display()))?;

    println!("走査中: {}", root.display());

    let mut summary = Summary::default();
    walk(platform, store, &root, recursive, 0, &mut summary, hash);

    println!();
    println!("  検出      : {} ファイル", summary.seen);
    println!("  新規記録  : {}", summary.added);
    println!("  変更なし  : {}", summary.unchanged);
    if summary.moved > 0 {
        println!("  移動検出  : {}", summary.moved);
    }
    if summary.copied > 0 {
        println!("  コピー検出: {}", summary.copied);
    }
    if summary.updated > 0 {
        println!("  内容更新  : {}", summary.updated);
    }
    println!("  入手元取得: {}", summary.origins_found);
    if summary.errors > 0 {
        println!("  エラー    : {}（権限不足など。処理は継続しました）", summary.errors);
    }

    // 入手元を読みに行くのは新規記録のときだけなので、
    // 新規が 0 件なら「取れなかった」と言うのは筋違い。
    if summary.origins_found == 0 && summary.added > 0 {
        println!();
        println!("新規 {} 件のうち、入手元が取れたものはありませんでした。", summary.added);
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
        if record_one(platform, store, &path, &meta, hash, summary).is_err() {
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
    let mtime = mtime_secs(meta);

    let sha256 = if hash {
        Some(fo_core::hash::sha256_file(path)?)
    } else {
        None
    };

    let observed = Observed {
        stable_id: stable_id.clone(),
        path: path.to_path_buf(),
        size: meta.len(),
        sha256,
    };

    // 突き合わせ候補: 識別子が一致するもの ＋ ハッシュが一致するもの。
    // どちらで捕まるかで「同一 / 移動 / コピー」が分かれる。
    let mut known = Vec::new();
    if let Some(rec) = store.find_by_stable_id(&stable_id)? {
        known.push(rec);
    }
    if let Some(d) = &observed.sha256 {
        known.extend(store.find_by_sha256(d)?);
    }

    match classify(&observed, &known, |p| p.exists()) {
        Verdict::Same { file_id } => {
            // 同一ボリューム内の移動・リネームはここに来る（識別子が変わらないため）。
            // パスが違えば、それが移動の記録になる。
            let current = known.iter().find(|r| r.id == file_id).map(|r| &r.current_path);
            if current.is_some_and(|p| p != path) {
                store.record_path(file_id, path)?;
                summary.moved += 1;
            } else {
                summary.unchanged += 1;
            }
            // 未計算だったハッシュを今回計算したなら埋める。
            if let (Some(d), Some(rec)) =
                (&observed.sha256, known.iter().find(|r| r.id == file_id))
            {
                if rec.sha256.is_none() {
                    store.set_sha256(file_id, d)?;
                }
            }
        }

        Verdict::Moved { file_id, .. } => {
            // 別ボリュームへの移動。識別子が変わったので差し替える。
            store.update_stable_id(file_id, &stable_id)?;
            store.record_path(file_id, path)?;
            summary.moved += 1;
        }

        Verdict::Updated { file_id } => {
            let d = observed
                .sha256
                .as_ref()
                .expect("Updated はハッシュ比較の結果なので必ずある");
            store.update_content(file_id, d, observed.size, mtime)?;
            if known.iter().any(|r| r.id == file_id && r.current_path != path) {
                store.record_path(file_id, path)?;
            }
            summary.updated += 1;
        }

        Verdict::Copied { from_file_id } => {
            let file_id = store.insert_file(
                &stable_id,
                path,
                observed.size,
                observed.sha256.as_ref(),
                mtime,
                Some(from_file_id),
            )?;
            summary.copied += 1;
            // コピーにも OS メタデータが付いていることがある（Explorer は ADS をコピーする）。
            summary.origins_found += import_os_origins(platform, store, path, file_id, mtime)?;
        }

        Verdict::New => {
            let file_id = store.insert_file(
                &stable_id,
                path,
                observed.size,
                observed.sha256.as_ref(),
                mtime,
                None,
            )?;
            summary.added += 1;
            summary.origins_found += import_os_origins(platform, store, path, file_id, mtime)?;
        }
    }

    Ok(())
}

/// OS が持っていた入手元を取り込む。取れた件数を返す。
///
/// 読めないこと自体はエラーにしない。ファイルの記録は成立しているため。
fn import_os_origins(
    platform: &dyn Platform,
    store: &Store,
    path: &Path,
    file_id: i64,
    mtime: i64,
) -> Result<usize> {
    let Ok(os_origins) = platform.origin_meta().read_origin(path) else {
        return Ok(0);
    };
    let mut n = 0;
    for os_origin in os_origins {
        let origin = Origin::from_os(os_origin, Some(mtime));
        store.add_origin(file_id, &origin)?;
        n += 1;
    }
    Ok(n)
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
