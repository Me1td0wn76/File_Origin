//! Linux の入手元メタデータ。
//!
//! △ **Windows と違い、この経路は当てにならない。**
//!
//! freedesktop.org は `user.xdg.origin.url` を標準として定義しているが、実装状況が悪い:
//!
//! | 実装 | 状況 |
//! |---|---|
//! | Firefox | × xattr を書かない（GVFS メタデータに書く） |
//! | Chrome / Chromium | × 実装後に撤回 |
//! | `wget --xattr` / `curl --xattr` | ○ 書く |
//!
//! そのため **Linux ではブラウザ拡張（M4）が事実上の必須機能**になる。
//! 調査の詳細は `docs/prior-art.md` §2.3 を参照。

use std::path::Path;

use crate::{Capability, Error, OriginMetadata, OsOrigin, OsOriginSource, Result};

const XATTR_ORIGIN: &str = "user.xdg.origin.url";
const XATTR_REFERRER: &str = "user.xdg.referrer.url";

pub struct LinuxOriginMetadata {
    /// GVFS メタデータを読むか。既定 OFF（ADR-0008）。
    ///
    /// Firefox はプライベートブラウジング中のダウンロードも GVFS に記録するため、
    /// ユーザーが記録されると思っていないものを拾ってしまう。黙って読まない。
    gvfs_enabled: bool,
}

impl LinuxOriginMetadata {
    pub fn new() -> Self {
        Self {
            gvfs_enabled: false,
        }
    }

    // TODO(M2): 設定から読んで有効化する経路を繋ぐ。それまで呼び出し元が無い。
    #[allow(dead_code)]
    pub fn with_gvfs(enabled: bool) -> Self {
        Self {
            gvfs_enabled: enabled,
        }
    }
}

impl Default for LinuxOriginMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginMetadata for LinuxOriginMetadata {
    fn read_origin(&self, path: &Path) -> Result<Vec<OsOrigin>> {
        let mut out = Vec::new();

        let url = read_xattr(path, XATTR_ORIGIN)?;
        let referrer = read_xattr(path, XATTR_REFERRER)?;

        if url.is_some() || referrer.is_some() {
            out.push(OsOrigin {
                url,
                referrer_url: referrer,
                source: OsOriginSource::Xattr,
                raw: None,
            });
        }

        if self.gvfs_enabled {
            // TODO(M2): ~/.local/share/gvfs-metadata/main.db を読み、
            // metadata::download-uri を取り出す。ADR-0008 を参照。
            // 有効化時はプライバシー警告を出すこと。
        }

        Ok(out)
    }

    fn available_sources(&self) -> Vec<(OsOriginSource, Capability)> {
        vec![
            (
                OsOriginSource::Xattr,
                // 読めることと、中身があることは別。主要ブラウザが書かないので、
                // 「使えるが期待するな」を伝える必要がある。
                Capability::Available,
            ),
            (
                OsOriginSource::Gvfs,
                if self.gvfs_enabled {
                    Capability::Available
                } else {
                    Capability::OptIn {
                        how: "設定で有効化（プライベートブラウジングの記録を含む可能性あり）",
                    }
                },
            ),
        ]
    }

    fn write_origin(&self, _path: &Path, _origin: &OsOrigin) -> Result<()> {
        // 設計方針 P6: 既定ではユーザーのファイルを書き換えない。
        Err(Error::Unsupported(
            "OS メタデータへの書き戻しは未実装（既定で無効の方針）",
        ))
    }
}

/// 拡張属性を 1 つ読む。
///
/// 属性が無いのは異常ではないので `None` を返す。
/// ファイルシステムが xattr 非対応（`ENOTSUP`）の場合も同じ扱いにする —
/// 呼び出し側から見れば「記録が無い」で挙動が変わらないため。
fn read_xattr(path: &Path, name: &str) -> Result<Option<String>> {
    match xattr::get(path, name) {
        Ok(Some(bytes)) => {
            let s = String::from_utf8_lossy(&bytes).trim().to_string();
            Ok(if s.is_empty() { None } else { Some(s) })
        }
        Ok(None) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => {
            // ENOTSUP には対応する ErrorKind が無いので errno を直接見る。
            // Linux では ENOTSUP == EOPNOTSUPP == 95。
            const ENOTSUP: i32 = 95;
            if e.raw_os_error() == Some(ENOTSUP) {
                Ok(None)
            } else {
                Err(Error::Io(e))
            }
        }
    }
}
