//! Native Messaging ホストの登録。
//!
//! ブラウザは「どの実行ファイルを、どの拡張に対して起動してよいか」を
//! **マニフェスト JSON** で知る。その置き場所が OS とブラウザで違うため、
//! ここを抽象化する（README §7.2）。
//!
//! | | Chrome 系 | Firefox |
//! |---|---|---|
//! | Windows | レジストリ `HKCU\Software\Google\Chrome\NativeMessagingHosts\<name>` | `HKCU\Software\Mozilla\NativeMessagingHosts\<name>` |
//! | Linux | `~/.config/google-chrome/NativeMessagingHosts/<name>.json` | `~/.mozilla/native-messaging-hosts/<name>.json` |
//!
//! Windows でもマニフェスト**ファイル自体**は要る。レジストリ値がその
//! ファイルのパスを指す、という二段構えになっている。

use std::path::PathBuf;

use crate::Result;

/// ホスト名。拡張の `connectNative()` に渡す文字列と一致させる。
///
/// Native Messaging の制約で **小文字・数字・アンダースコア・ドット** のみ。
/// ハイフンは使えないので `file_origin`（`file-origin` ではない）。
pub const HOST_NAME: &str = "io.github.file_origin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Edge,
    Chromium,
    Firefox,
}

impl Browser {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "chrome" => Some(Self::Chrome),
            "edge" => Some(Self::Edge),
            "chromium" => Some(Self::Chromium),
            "firefox" => Some(Self::Firefox),
            _ => None,
        }
    }

    pub fn all() -> &'static [Browser] {
        &[Self::Chrome, Self::Edge, Self::Chromium, Self::Firefox]
    }

    /// Firefox だけマニフェストの鍵が違う。
    ///
    /// Chrome 系は拡張 ID を `allowed_origins`（`chrome-extension://<id>/`）、
    /// Firefox は `allowed_extensions`（`<id>@example`）で指定する。
    pub fn is_firefox(self) -> bool {
        matches!(self, Self::Firefox)
    }
}

/// マニフェストに書く内容。
#[derive(Debug, Clone)]
pub struct HostManifest {
    /// 中継プロセスの絶対パス。
    pub exe_path: PathBuf,
    /// 接続を許可する拡張の識別子。**空にしない** —
    /// 空だとどの拡張からも繋げてしまい、任意のページ由来の拡張が
    /// ダウンロード記録を偽装できる。
    pub allowed: Vec<String>,
}

impl HostManifest {
    /// ブラウザ向けの JSON にする。
    pub fn to_json(&self, browser: Browser) -> String {
        let key = if browser.is_firefox() {
            "allowed_extensions"
        } else {
            "allowed_origins"
        };
        let allowed: Vec<String> = self
            .allowed
            .iter()
            .map(|id| {
                if browser.is_firefox() {
                    id.clone()
                } else {
                    // Chrome 系は origin 形式（末尾のスラッシュまで含めて一致判定される）
                    format!("chrome-extension://{id}/")
                }
            })
            .collect();

        let value = serde_json_lite(&[
            ("name", Json::Str(HOST_NAME.to_string())),
            (
                "description",
                Json::Str("File Origin: ダウンロードの入手元を記録します".to_string()),
            ),
            (
                "path",
                Json::Str(self.exe_path.to_string_lossy().into_owned()),
            ),
            ("type", Json::Str("stdio".to_string())),
            (key, Json::Arr(allowed)),
        ]);
        value
    }
}

/// 登録の結果。どこに何を置いたかを利用者に見せるため。
#[derive(Debug, Clone)]
pub struct Installed {
    pub browser: Browser,
    /// 置いたマニフェストファイル。
    pub manifest_path: PathBuf,
    /// Windows のレジストリキーなど、ファイル以外に書いた場所。
    pub registry_key: Option<String>,
}

pub trait NativeHostInstaller {
    /// マニフェストを設置する。
    fn install(&self, browser: Browser, manifest: &HostManifest) -> Result<Installed>;
    /// 設置を取り消す。存在しなければ何もしない。
    fn uninstall(&self, browser: Browser) -> Result<()>;
    /// 設置済みか。
    fn is_installed(&self, browser: Browser) -> bool;
    /// マニフェストの置き場所（未設置でも答えられる）。
    fn manifest_path(&self, browser: Browser) -> PathBuf;
}

// ---------------------------------------------------------------------------
// 小さな JSON 生成
// ---------------------------------------------------------------------------
// マニフェストは固定の形しか書かないので、serde を持ち込まずに済ませる。
// fo-platform に serde を足すと、OS 抽象化層の依存が増えて見通しが悪くなる。

enum Json {
    Str(String),
    Arr(Vec<String>),
}

fn serde_json_lite(fields: &[(&str, Json)]) -> String {
    let mut out = String::from("{\n");
    for (i, (k, v)) in fields.iter().enumerate() {
        out.push_str("  \"");
        out.push_str(k);
        out.push_str("\": ");
        match v {
            Json::Str(s) => {
                out.push('"');
                out.push_str(&escape(s));
                out.push('"');
            }
            Json::Arr(items) => {
                out.push('[');
                for (j, it) in items.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    out.push('"');
                    out.push_str(&escape(it));
                    out.push('"');
                }
                out.push(']');
            }
        }
        if i + 1 < fields.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// JSON 文字列の最小限のエスケープ。
/// Windows のパスに含まれるバックスラッシュを必ず二重にする。
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> HostManifest {
        HostManifest {
            exe_path: PathBuf::from(r"C:\Program Files\FileOrigin\fo-nativehost.exe"),
            allowed: vec!["abcdefghijklmnopabcdefghijklmnop".to_string()],
        }
    }

    #[test]
    fn chrome_uses_allowed_origins() {
        let json = manifest().to_json(Browser::Chrome);
        assert!(json.contains("\"allowed_origins\""));
        assert!(json.contains("chrome-extension://abcdefghijklmnopabcdefghijklmnop/"));
        assert!(!json.contains("allowed_extensions"));
    }

    #[test]
    fn firefox_uses_allowed_extensions_without_scheme() {
        let json = manifest().to_json(Browser::Firefox);
        assert!(json.contains("\"allowed_extensions\""));
        assert!(!json.contains("chrome-extension://"));
    }

    #[test]
    fn windows_paths_are_escaped() {
        // ここを間違えるとブラウザがマニフェストを読めず、
        // 「拡張は動くのに何も記録されない」という分かりにくい壊れ方をする。
        let json = manifest().to_json(Browser::Chrome);
        assert!(json.contains(r"C:\\Program Files\\FileOrigin\\fo-nativehost.exe"));
    }

    #[test]
    fn host_name_is_valid_for_native_messaging() {
        // 小文字・数字・アンダースコア・ドットのみ。ハイフン不可。
        assert!(HOST_NAME
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'));
    }

    #[test]
    fn browser_names_round_trip() {
        for b in Browser::all() {
            assert_eq!(Browser::parse(b.as_str()), Some(*b));
        }
        assert_eq!(Browser::parse("safari"), None);
    }
}
