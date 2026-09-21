//! 1 ファイルを取り込む。
//!
//! 観測したファイルが既知のどれに対応するかは `fo_core::classify` に委ねる。
//! ここで独自の判定を書くと、テスト済みのはしごと挙動がずれる。

use std::path::Path;

use fo_core::model::Origin;
use fo_core::{classify, Observed, Verdict};
use fo_platform::Platform;
use fo_store::Store;

use crate::Result;

#[derive(Debug, Clone, Copy, Default)]
pub struct IngestOptions {
    /// SHA-256 を同時に計算する。既定は計算しない（ADR-0007）。
    /// 安定識別子だけで追跡は始められるので、大きいファイルで待たせる理由がない。
    pub hash: bool,
}

/// 取り込みの結果。呼び出し側はこれを見て集計や表示をする。
#[derive(Debug)]
pub struct Ingested {
    pub file_id: i64,
    pub verdict: Verdict,
    /// OS メタデータから取り込めた入手元の件数。新規・コピーのときだけ読みに行く。
    pub os_origins_recorded: usize,
}

/// ファイルを DB に取り込む。既知なら状態を更新し、未知なら新規に記録する。
///
/// `path` は正規化済みであること（`PlatformPaths::canonical` を通す）。
/// ここで正規化しないのは、ディレクトリ走査で親を 1 回正規化すれば
/// 子は全部正規化済みになり、ファイルごとに syscall を払わずに済むため。
pub fn ingest_file(
    platform: &dyn Platform,
    store: &Store,
    path: &Path,
    meta: &std::fs::Metadata,
    opts: IngestOptions,
) -> Result<Ingested> {
    let stable_id = platform.identity().stable_id(path)?;
    let mtime = mtime_secs(meta);

    let sha256 = if opts.hash {
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

    let verdict = classify(&observed, &known, |p| p.exists());
    let mut os_origins_recorded = 0;

    let file_id = match &verdict {
        Verdict::Same { file_id } => {
            let file_id = *file_id;
            let rec = known.iter().find(|r| r.id == file_id);
            // 同一ボリューム内の移動・リネームはここに来る（識別子が変わらないため）。
            // パスが違えば、それが移動の記録になる。
            if rec.is_some_and(|r| r.current_path != path) {
                store.record_path(file_id, path)?;
            }
            // 未計算だったハッシュを今回計算したなら埋める。
            if let (Some(d), Some(r)) = (&observed.sha256, rec) {
                if r.sha256.is_none() {
                    store.set_sha256(file_id, d)?;
                }
            }
            file_id
        }

        Verdict::Moved { file_id, .. } => {
            // 別ボリュームへの移動。識別子が変わったので差し替える。
            store.update_stable_id(*file_id, &stable_id)?;
            store.record_path(*file_id, path)?;
            *file_id
        }

        Verdict::Updated { file_id } => {
            let d = observed
                .sha256
                .as_ref()
                .expect("Updated はハッシュ比較の結果なので必ずある");
            store.update_content(*file_id, d, observed.size, mtime)?;
            if known
                .iter()
                .any(|r| r.id == *file_id && r.current_path != path)
            {
                store.record_path(*file_id, path)?;
            }
            *file_id
        }

        Verdict::Copied { from_file_id } => {
            let file_id = store.insert_file(
                &stable_id,
                path,
                observed.size,
                observed.sha256.as_ref(),
                mtime,
                Some(*from_file_id),
            )?;
            // コピーにも OS メタデータが付いていることがある（Explorer は ADS をコピーする）。
            os_origins_recorded = import_os_origins(platform, store, path, file_id, mtime)?;
            file_id
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
            os_origins_recorded = import_os_origins(platform, store, path, file_id, mtime)?;
            file_id
        }
    };

    Ok(Ingested {
        file_id,
        verdict,
        os_origins_recorded,
    })
}

/// 同じことを、パスだけ渡して行う。単発の `fo add` / `fo show` 向け。
/// 正規化と metadata の取得を代行する。
pub fn ingest_path(
    platform: &dyn Platform,
    store: &Store,
    path: &Path,
    opts: IngestOptions,
) -> Result<Ingested> {
    let canon = platform
        .paths()
        .canonical(path)
        .map_err(|source| crate::Error::Canonicalize {
            path: path.to_path_buf(),
            source,
        })?;
    let meta = std::fs::metadata(&canon)?;
    ingest_file(platform, store, &canon, &meta, opts)
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

pub(crate) fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
