//! Windows の入手元メタデータ = NTFS 代替データストリーム `<file>:Zone.Identifier`。
//!
//! 主要ブラウザとメールクライアントが `IAttachmentExecute` 経由で自動的に書くため、
//! **Windows では拡張機能を入れる前のファイルも救済できる**。
//! （Linux にこれに相当する信頼できる経路は無い。README §9 の注記を参照）
//!
//! 中身は INI 形式:
//! ```text
//! [ZoneTransfer]
//! ZoneId=3
//! ReferrerUrl=https://example.com/download-page
//! HostUrl=https://cdn.example.com/files/setup.zip
//! ```
//!
//! ADS はファイルシステムから普通のファイルとして読めるので FFI は要らない。
//!
//! パースの注意点（64 KiB 上限・BOM・エンコーディング）は
//! 先行実装 [WhereFrom](https://github.com/opsorart/WhereFrom)（MIT）の
//! ドキュメントに挙げられていたものを参考にした。

use std::path::Path;

use crate::{Capability, OriginMetadata, OsOrigin, OsOriginSource, Result};

/// Zone.Identifier の読み取り上限。
/// ADS は任意長を持てるため、壊れた / 細工されたストリームで
/// メモリを食い潰さないよう頭を押さえる。
const MAX_ZONE_BYTES: usize = 64 * 1024;

pub struct WindowsOriginMetadata;

impl OriginMetadata for WindowsOriginMetadata {
    fn read_origin(&self, path: &Path) -> Result<Vec<OsOrigin>> {
        let mut ads = path.as_os_str().to_os_string();
        ads.push(":Zone.Identifier");

        let bytes = match std::fs::read(&ads) {
            Ok(b) => b,
            // ストリームが無い = このファイルには記録が無い。異常ではない。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let text = decode(&bytes);
        Ok(parse_zone_identifier(&text).into_iter().collect())
    }

    fn available_sources(&self) -> Vec<(OsOriginSource, Capability)> {
        vec![(OsOriginSource::ZoneIdentifier, Capability::Available)]
    }

    fn write_origin(&self, _path: &Path, _origin: &OsOrigin) -> Result<()> {
        // 設計方針 P6: 既定ではユーザーのファイルを書き換えない。
        // オプトインで有効化する機能として M6 以降に検討する。
        Err(crate::Error::Unsupported(
            "OS メタデータへの書き戻しは未実装（既定で無効の方針）",
        ))
    }
}

/// BOM を落として UTF-8 として解釈する。
///
/// Zone.Identifier は通常 ASCII だが、UTF-8 BOM 付きで書かれることがある。
/// 不正なバイトは lossy に潰す — URL が 1 文字化けても、記録が無いよりはよい。
fn decode(bytes: &[u8]) -> String {
    let capped = &bytes[..bytes.len().min(MAX_ZONE_BYTES)];
    let body = capped.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(capped);
    String::from_utf8_lossy(body).into_owned()
}

/// `[ZoneTransfer]` セクションから入手元を取り出す。
///
/// `HostUrl` が実際の取得先、`ReferrerUrl` が人間が見ていたページ。
/// 両方揃わないことが多いので、片方でもあれば記録する。
///
/// 実データで分かった 2 つの慣用形を扱う:
///
/// - **`about:internet`** — URL を記録できなかったときに Windows が書く値。
///   「インターネット由来」以上の情報が無いので、URL としては捨てる。
/// - **`ReferrerUrl=C:\...\archive.zip`** — 書庫から展開したファイルには、
///   URL ではなく **展開元の書庫のローカルパス** が入る。これは来歴そのもの
///   （「この DLL はどの zip から出てきたか」）なので、`file://` URL に正規化して
///   取得先として記録する。Downloads の中身の大半はこの形になる。
fn parse_zone_identifier(text: &str) -> Option<OsOrigin> {
    let mut host_url = None;
    let mut referrer_url = None;
    let mut in_zone_transfer = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_zone_transfer = line.eq_ignore_ascii_case("[ZoneTransfer]");
            continue;
        }
        if !in_zone_transfer {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }

        match key.trim().to_ascii_lowercase().as_str() {
            "hosturl" => host_url = normalize_zone_url(value),
            "referrerurl" => referrer_url = normalize_zone_url(value),
            _ => {}
        }
    }

    // HostUrl が無く ReferrerUrl が書庫のローカルパスなら、取得先はその書庫。
    // 「参照元」の枠に置いたままだと、URL 無しの記録として検索から漏れる。
    let (url, referrer_url) = match (host_url, referrer_url) {
        (None, Some(r)) if r.starts_with("file://") => (Some(r), None),
        pair => pair,
    };

    if url.is_none() && referrer_url.is_none() {
        // ZoneId しか無いケース。「インターネット由来」とは分かるが URL が無いので、
        // 入手元としては記録しない。ここで空の Origin を作ると、
        // 「記録がある」と「URL が分からない」が混ざってしまう。
        return None;
    }

    Some(OsOrigin {
        url,
        referrer_url,
        source: OsOriginSource::ZoneIdentifier,
        raw: Some(text.to_string()),
    })
}

/// Zone.Identifier の URL 値を正規化する。
///
/// - `about:*`（`about:internet` など）→ 情報が無いので `None`
/// - ローカルパス（`C:\...` / `\\server\share\...`）→ `file://` URL
/// - それ以外はそのまま
fn normalize_zone_url(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if lower.starts_with("about:") {
        return None;
    }
    if let Some(url) = local_path_to_file_url(v) {
        return Some(url);
    }
    Some(v.to_string())
}

/// Windows のローカルパスを `file://` URL にする。パスでなければ `None`。
///
/// `C:\a\b` → `file:///C:/a/b`、`\\nas\share\x` → `file://nas/share/x`。
/// パーセントエンコードはしない — 検索で部分一致させたいので、人間が読む形のままにする。
fn local_path_to_file_url(v: &str) -> Option<String> {
    let bytes = v.as_bytes();
    let is_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if is_drive {
        return Some(format!("file:///{}", v.replace('\\', "/")));
    }
    if let Some(unc) = v.strip_prefix(r"\\") {
        if !unc.is_empty() && !unc.starts_with('\\') {
            return Some(format!("file://{}", unc.replace('\\', "/")));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_referrer() {
        let text = "[ZoneTransfer]\r\nZoneId=3\r\n\
                    ReferrerUrl=https://example.com/page\r\n\
                    HostUrl=https://cdn.example.com/setup.zip\r\n";
        let origin = parse_zone_identifier(text).expect("入手元が取れるはず");
        assert_eq!(
            origin.url.as_deref(),
            Some("https://cdn.example.com/setup.zip")
        );
        assert_eq!(
            origin.referrer_url.as_deref(),
            Some("https://example.com/page")
        );
        assert_eq!(origin.source, OsOriginSource::ZoneIdentifier);
    }

    #[test]
    fn about_internet_is_dropped() {
        // URL を記録できなかったときの慣用値。情報が無いので記録しない。
        let text = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=about:internet\r\n";
        assert!(parse_zone_identifier(text).is_none());

        // 参照元があるなら、それだけは残す。
        let text = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=about:internet\r\n\
                    ReferrerUrl=https://example.com/page\r\n";
        let o = parse_zone_identifier(text).unwrap();
        assert_eq!(o.url, None);
        assert_eq!(o.referrer_url.as_deref(), Some("https://example.com/page"));
    }

    #[test]
    fn extracted_from_archive_becomes_file_url_source() {
        // 書庫から展開したファイル: ReferrerUrl に書庫のローカルパスが入る。
        let text = "[ZoneTransfer]\r\nZoneId=3\r\n\
                    ReferrerUrl=C:\\Users\\me\\Downloads\\pack.7z\r\n";
        let o = parse_zone_identifier(text).unwrap();
        // 取得先 = 書庫。参照元の枠には残さない。
        assert_eq!(
            o.url.as_deref(),
            Some("file:///C:/Users/me/Downloads/pack.7z")
        );
        assert_eq!(o.referrer_url, None);
    }

    #[test]
    fn local_path_conversion() {
        assert_eq!(
            local_path_to_file_url(r"D:\x\y.zip").as_deref(),
            Some("file:///D:/x/y.zip")
        );
        assert_eq!(
            local_path_to_file_url(r"\\nas\share\a.zip").as_deref(),
            Some("file://nas/share/a.zip")
        );
        assert_eq!(local_path_to_file_url("https://example.com/a"), None);
        assert_eq!(local_path_to_file_url("C:"), None);
    }

    #[test]
    fn zone_id_only_yields_nothing() {
        // URL が無いなら入手元の記録は作らない。
        assert!(parse_zone_identifier("[ZoneTransfer]\r\nZoneId=3\r\n").is_none());
    }

    #[test]
    fn ignores_keys_outside_zone_transfer() {
        let text = "[Other]\r\nHostUrl=https://evil.example/\r\n\
                    [ZoneTransfer]\r\nHostUrl=https://good.example/\r\n";
        let origin = parse_zone_identifier(text).unwrap();
        assert_eq!(origin.url.as_deref(), Some("https://good.example/"));
    }

    #[test]
    fn strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"[ZoneTransfer]\r\nHostUrl=https://example.com/a\r\n");
        let origin = parse_zone_identifier(&decode(&bytes)).unwrap();
        assert_eq!(origin.url.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn caps_oversized_stream() {
        let huge = vec![b'x'; MAX_ZONE_BYTES * 2];
        assert_eq!(decode(&huge).len(), MAX_ZONE_BYTES);
    }
}
