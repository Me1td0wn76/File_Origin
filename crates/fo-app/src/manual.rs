//! 入手元を手で登録する。`fo add --url`。

use std::path::Path;

use fo_core::model::{host_of, Origin, OriginSource};
use fo_platform::Platform;
use fo_store::Store;

use crate::ingest::{ingest_path, IngestOptions, Ingested};
use crate::Result;

/// ファイルを（未記録なら）取り込み、手動入力の入手元を 1 件積む。
///
/// 既に同じ URL が別経路（Zone.Identifier など）で記録されていても上書きしない。
/// 経路ごとに 1 行あることで「OS の記録と本人の申告が一致した」と分かる。
/// 矛盾していても両方残す — File Origin は判定機ではなく記録装置。
pub fn add_manual_origin(
    platform: &dyn Platform,
    store: &Store,
    path: &Path,
    url: &str,
    referrer_url: Option<&str>,
) -> Result<(Ingested, i64)> {
    let ingested = ingest_path(platform, store, path, IngestOptions::default())?;

    let origin = Origin {
        url: Some(url.to_string()),
        referrer_url: referrer_url.map(str::to_string),
        host: host_of(url),
        acquired_at: None,
        source: OriginSource::Manual,
        // 本人の申告は確定扱い。間違っていたとしてもそれは本人の責任範囲で、
        // ツールが勝手に格下げする根拠が無い。
        confidence: OriginSource::Manual.default_confidence(),
        browser: None,
        profile: None,
    };
    let origin_id = store.add_origin(ingested.file_id, &origin)?;
    Ok((ingested, origin_id))
}
