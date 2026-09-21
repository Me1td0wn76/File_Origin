//! ファイルの来歴を引く。`fo show` と GUI の詳細画面が使う。

use std::path::Path;

use fo_core::model::{FileRecord, Origin, PathEntry};
use fo_platform::Platform;
use fo_store::Store;

use crate::{Error, Result};

/// `derived_from` を辿る上限。コピーのコピーのコピー……が
/// 万一 DB の不整合で輪になっていても止まるように。
const MAX_LINEAGE: usize = 16;

#[derive(Debug)]
pub struct Description {
    pub record: FileRecord,
    /// 新しい順。先頭が現在のパス。
    pub paths: Vec<PathEntry>,
    /// このファイル自身に記録された入手元。確度の高い順。
    pub origins: Vec<Origin>,
    /// コピー元を近い順に辿ったもの。コピーでなければ空。
    pub lineage: Vec<FileRecord>,
    /// 祖先に記録された入手元。`(祖先の id, 入手元)`。近い祖先から順。
    ///
    /// コピーには入手元が付いていないことが多い（cp は ADS を落とす）。
    /// その場合でも系譜を辿れば出所は分かる — それが `derived_from` を持つ理由。
    pub inherited_origins: Vec<(i64, Origin)>,
}

/// パスから来歴を引く。記録が無ければ `Err(NotRecorded)`。
pub fn describe(platform: &dyn Platform, store: &Store, path: &Path) -> Result<Description> {
    // scan と同じ正規化を通してから引く。ここがずれると同じファイルが見つからない。
    let canon = platform
        .paths()
        .canonical(path)
        .unwrap_or_else(|_| path.to_path_buf());

    let record = store
        .find_by_path(&canon)?
        .ok_or_else(|| Error::NotRecorded(canon.clone()))?;

    describe_record(store, record)
}

pub fn describe_record(store: &Store, record: FileRecord) -> Result<Description> {
    let paths = store.path_history(record.id)?;
    let origins = store.origins_of(record.id)?;

    let mut lineage = Vec::new();
    let mut inherited_origins = Vec::new();
    let mut next = record.derived_from;
    while let Some(parent_id) = next {
        if lineage.len() >= MAX_LINEAGE {
            break;
        }
        let Some(parent) = store.get_file(parent_id)? else {
            break;
        };
        for o in store.origins_of(parent.id)? {
            inherited_origins.push((parent.id, o));
        }
        next = parent.derived_from;
        lineage.push(parent);
    }

    Ok(Description {
        record,
        paths,
        origins,
        lineage,
        inherited_origins,
    })
}
