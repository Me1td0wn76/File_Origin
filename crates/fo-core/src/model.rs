//! ドメインモデル。OS を一切知らない。

use std::path::PathBuf;

use fo_platform::{OsOriginSource, StableFileId};

/// SHA-256 ダイジェスト（16 進小文字）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Digest(pub String);

impl Digest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 追跡対象ファイルの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// 記録した場所に実在する。
    Present,
    /// 見失っている。再スキャンで戻る可能性がある。
    Missing,
    /// 削除が確認された。
    Deleted,
}

/// 入手元をどの経路で得たか。
///
/// `OsOriginSource`（OS が持っていたもの）に、OS の関知しない経路を足したもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginSource {
    /// ブラウザ拡張が Native Messaging で報告したもの。最も豊富で正確。
    BrowserExt,
    /// Windows: NTFS `Zone.Identifier`
    ZoneIdentifier,
    /// Linux: 拡張属性 `user.xdg.origin.url`
    Xattr,
    /// Linux: GVFS メタデータ（オプトイン）
    Gvfs,
    /// ユーザーが手で入力したもの。
    Manual,
    /// ブラウザの履歴 DB との突き合わせ（オプトイン）。
    HistoryDb,
}

impl OriginSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BrowserExt => "browser_ext",
            Self::ZoneIdentifier => "zone_identifier",
            Self::Xattr => "xattr",
            Self::Gvfs => "gvfs",
            Self::Manual => "manual",
            Self::HistoryDb => "history_db",
        }
    }

    /// その経路から得た入手元の既定の確度。
    ///
    /// 迷ったら低い方に倒す。File Origin の価値は記録が信用できることにあるので、
    /// 確度を盛ると価値そのものが壊れる。
    pub fn default_confidence(self) -> Confidence {
        match self {
            Self::BrowserExt | Self::Manual => Confidence::Certain,
            Self::ZoneIdentifier | Self::Xattr => Confidence::High,
            Self::Gvfs | Self::HistoryDb => Confidence::Medium,
        }
    }
}

impl From<OsOriginSource> for OriginSource {
    fn from(s: OsOriginSource) -> Self {
        match s {
            OsOriginSource::ZoneIdentifier => Self::ZoneIdentifier,
            OsOriginSource::Xattr => Self::Xattr,
            OsOriginSource::Gvfs => Self::Gvfs,
        }
    }
}

/// 記録をどれくらい信じてよいか。
///
/// `source` が「どこから取ったか」、`confidence` が「どれくらい信じてよいか」。
/// 両方ないと、矛盾する情報が来たときにどちらを優先するか決められない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// 候補として提示するだけ。自動確定しない。
    Low,
    /// 状況証拠から結びつけたもの。取り違えがありうる。
    Medium,
    /// OS やツールが記録したもので、通常は正しい。
    High,
    /// そのファイルの入手元であることが確定している。
    Certain,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Certain => "certain",
        }
    }
}

/// 入手元 1 件。1 ファイルに複数ぶら下がる。
///
/// 再ダウンロードしても上書きせず積む。矛盾する情報が来ても握りつぶさない —
/// File Origin は判定機ではなく記録装置なので、確度を添えて両方残す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub url: Option<String>,
    pub referrer_url: Option<String>,
    /// URL から導出したホスト名。ホスト単位の検索に使う。
    pub host: Option<String>,
    /// 取得日時（Unix 秒）。分からなければ `None`。
    pub acquired_at: Option<i64>,
    pub source: OriginSource,
    pub confidence: Confidence,
    pub browser: Option<String>,
    pub profile: Option<String>,
}

impl Origin {
    pub fn from_os(os: fo_platform::OsOrigin, acquired_at: Option<i64>) -> Self {
        let source = OriginSource::from(os.source);
        let host = os.url.as_deref().and_then(host_of);
        Self {
            url: os.url,
            referrer_url: os.referrer_url,
            host,
            acquired_at,
            source,
            confidence: source.default_confidence(),
            browser: None,
            profile: None,
        }
    }
}

/// URL からホスト名を取り出す。
///
/// 完全な URL パーサは入れない。ホスト単位の検索に使うだけなので、
/// スキーム区切りと最初の `/` の間を取れば足りる。
/// パースに失敗したら `None` を返し、URL 自体は元のまま保持する。
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next()?;
    // 認証情報とポートを落とす
    let host = authority.rsplit('@').next()?;
    let host = match host.rfind(':') {
        // IPv6 リテラル `[::1]:8080` を壊さない
        Some(i) if !host.contains(']') || host.rfind(']').is_some_and(|b| b < i) => &host[..i],
        _ => host,
    };
    let host = host.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// パス履歴の 1 行。ファイルが「いつ・どこにあったか」。
///
/// パスは属性ではなく履歴として持つ（README §10）。移動・リネームのたびに
/// 行が増え、古い行は `is_current = false` で残る。消さない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub path: PathBuf,
    pub is_current: bool,
    /// このパスで観測した日時（Unix 秒）。
    pub observed_at: i64,
}

/// 追跡対象ファイル 1 件。
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: i64,
    pub stable_id: StableFileId,
    pub current_path: PathBuf,
    pub size: u64,
    /// 計算前は `None`。大容量ファイルは遅延計算する（ADR-0007）。
    pub sha256: Option<Digest>,
    pub mtime: i64,
    pub status: FileStatus,
    /// コピー元。コピーを別ファイルとして扱いつつ、入手元の系譜を辿れる。
    pub derived_from: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_host() {
        assert_eq!(
            host_of("https://cdn.Example.com/a/b.zip").as_deref(),
            Some("cdn.example.com")
        );
        assert_eq!(
            host_of("http://example.com:8080/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_of("https://user:pw@example.com/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_of("https://example.com").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn handles_unparseable_urls() {
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("https://"), None);
    }

    #[test]
    fn browser_ext_outranks_os_metadata() {
        // 確度の順序が意図どおりか。UI はこの順で入手元を並べる。
        assert!(
            OriginSource::BrowserExt.default_confidence()
                > OriginSource::ZoneIdentifier.default_confidence()
        );
        assert!(OriginSource::Xattr.default_confidence() > OriginSource::Gvfs.default_confidence());
    }
}
