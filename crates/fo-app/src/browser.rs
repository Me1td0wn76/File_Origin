//! ブラウザ拡張からのダウンロード報告を取り込む（M4 の受け口）。
//!
//! **これが入手元の本命経路。** OS メタデータと違い、リダイレクトを追った
//! 最終 URL・参照元ページ・取得時刻・ブラウザとプロファイルまで揃う。
//! だから確度は `certain`（`OriginSource::BrowserExt` の既定）。

use std::path::{Path, PathBuf};

use fo_core::model::{host_of, Origin, OriginSource};
use fo_platform::Platform;
use fo_store::Store;

use crate::ingest::{ingest_path, IngestOptions, Ingested};
use crate::{Error, Result};

/// ブラウザが報告したダウンロード 1 件。
///
/// `fo-ipc` の `DownloadReport` と同じ内容だが、こちらはアプリ層の型。
/// IPC の線表現に `fo-app` を縛られないよう分けてある
/// （拡張のプロトコルが変わっても、こちらは変えずに済む）。
#[derive(Debug, Clone)]
pub struct DownloadReport {
    pub path: PathBuf,
    pub url: Option<String>,
    pub referrer: Option<String>,
    pub acquired_at: Option<i64>,
    pub browser: Option<String>,
    pub profile: Option<String>,
}

/// 報告を取り込み、入手元を記録する。
///
/// ファイルがまだ無い場合は失敗する。拡張は `state === "complete"` を待って
/// 報告する約束なので、無いのは異常（一時ファイル名のまま報告された等）。
pub fn record_download(
    platform: &dyn Platform,
    store: &Store,
    report: &DownloadReport,
) -> Result<(Ingested, i64)> {
    if report.url.is_none() && report.referrer.is_none() {
        // URL の無い報告は記録しない。「記録がある」と「URL が分からない」を
        // 混ぜると、後で来歴を見たときに何も分からない行が残る。
        return Err(Error::EmptyReport);
    }

    let ingested = ingest_path(platform, store, &report.path, IngestOptions::default())?;

    let origin = Origin {
        url: report.url.clone(),
        referrer_url: report.referrer.clone(),
        host: report.url.as_deref().and_then(host_of),
        acquired_at: report.acquired_at,
        source: OriginSource::BrowserExt,
        confidence: OriginSource::BrowserExt.default_confidence(),
        browser: report.browser.clone(),
        profile: report.profile.clone(),
    };
    let origin_id = store.add_origin(ingested.file_id, &origin)?;
    Ok((ingested, origin_id))
}

/// 拡張が渡してくるパスを正規化する。
///
/// ブラウザは OS 形式の絶対パスを渡すが、`file:///C:/...` を渡す実装もある。
/// どちらでも受けられるようにしておく — 拡張側の実装差でデーモンが
/// 動かなくなるのは避けたい。
pub fn normalize_reported_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("file:///") {
        // Windows の `file:///C:/a` → `C:/a`、Unix の `file:///a` → `/a`
        let looks_like_drive = rest.len() >= 2 && rest.as_bytes()[1] == b':';
        return if looks_like_drive {
            PathBuf::from(rest)
        } else {
            PathBuf::from(format!("/{rest}"))
        };
    }
    PathBuf::from(raw)
}

/// 報告されたパスが取り込んでよいものか。
pub fn is_acceptable(path: &Path) -> bool {
    !crate::watch::is_in_progress(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_file_urls() {
        assert_eq!(
            normalize_reported_path("file:///C:/dl/a.zip"),
            PathBuf::from("C:/dl/a.zip")
        );
        assert_eq!(
            normalize_reported_path("file:///home/me/a.zip"),
            PathBuf::from("/home/me/a.zip")
        );
        assert_eq!(
            normalize_reported_path(r"C:\dl\a.zip"),
            PathBuf::from(r"C:\dl\a.zip")
        );
    }

    #[test]
    fn rejects_in_progress_names() {
        assert!(!is_acceptable(Path::new("/dl/a.zip.crdownload")));
        assert!(is_acceptable(Path::new("/dl/a.zip")));
    }
}
