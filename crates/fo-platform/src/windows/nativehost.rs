//! Windows の Native Messaging ホスト登録。
//!
//! 二段構え:
//!
//! 1. マニフェスト JSON を `%LOCALAPPDATA%\FileOrigin\` に置く
//! 2. レジストリ `HKCU\Software\...\NativeMessagingHosts\<name>` の既定値に
//!    そのファイルのパスを書く
//!
//! `HKCU` を使うので**管理者権限は要らない**（設計方針どおり）。
//! `HKLM` に書けば全ユーザーに効くが、昇格が必要になるので採らない。
//!
//! レジストリ操作は `reg.exe` に任せる。`windows-sys` で
//! `RegCreateKeyExW` を叩いてもよいが、ここは起動頻度が低く（設定時の 1 回だけ）、
//! unsafe を増やす価値がない。

use std::path::PathBuf;
use std::process::Command;

use crate::nativehost::{Browser, HostManifest, Installed, NativeHostInstaller, HOST_NAME};
use crate::{Error, PlatformPaths, Result};

pub struct WindowsHostInstaller<P> {
    paths: P,
}

impl<P: PlatformPaths> WindowsHostInstaller<P> {
    pub fn new(paths: P) -> Self {
        Self { paths }
    }

    /// ブラウザごとのレジストリキー。
    fn registry_key(browser: Browser) -> String {
        let vendor = match browser {
            Browser::Chrome => r"Software\Google\Chrome",
            Browser::Edge => r"Software\Microsoft\Edge",
            Browser::Chromium => r"Software\Chromium",
            Browser::Firefox => r"Software\Mozilla",
        };
        format!(r"HKCU\{vendor}\NativeMessagingHosts\{HOST_NAME}")
    }
}

impl<P: PlatformPaths> NativeHostInstaller for WindowsHostInstaller<P> {
    fn manifest_path(&self, browser: Browser) -> PathBuf {
        // ブラウザごとに別ファイルにする。allowed_origins の形式が
        // Chrome 系と Firefox で違うため、1 つに共用できない。
        self.paths
            .data_dir()
            .join("nativehost")
            .join(format!("{HOST_NAME}.{}.json", browser.as_str()))
    }

    fn install(&self, browser: Browser, manifest: &HostManifest) -> Result<Installed> {
        let path = self.manifest_path(browser);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, manifest.to_json(browser))?;

        let key = Self::registry_key(browser);
        let out = Command::new("reg")
            .args([
                "add",
                &key,
                "/ve", // 既定値
                "/t",
                "REG_SZ",
                "/d",
                &path.to_string_lossy(),
                "/f", // 既存を上書き
            ])
            .output()
            .map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "reg.exe を実行できません: {e}"
                )))
            })?;

        if !out.status.success() {
            return Err(Error::Io(std::io::Error::other(format!(
                "レジストリに書けません ({key}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))));
        }

        Ok(Installed {
            browser,
            manifest_path: path,
            registry_key: Some(key),
        })
    }

    fn uninstall(&self, browser: Browser) -> Result<()> {
        // キーが無くても失敗にしない。取り消しは冪等であるべき。
        let _ = Command::new("reg")
            .args(["delete", &Self::registry_key(browser), "/f"])
            .output();
        let path = self.manifest_path(browser);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn is_installed(&self, browser: Browser) -> bool {
        if !self.manifest_path(browser).exists() {
            return false;
        }
        Command::new("reg")
            .args(["query", &Self::registry_key(browser), "/ve"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_are_per_browser_and_user_scoped() {
        for b in Browser::all() {
            let key = WindowsHostInstaller::<crate::windows::paths::WindowsPaths>::registry_key(*b);
            // 昇格を避けるため HKCU のみ。HKLM に書くと管理者権限が要る。
            assert!(key.starts_with(r"HKCU\"), "{key}");
            assert!(key.ends_with(HOST_NAME), "{key}");
        }
    }

    #[test]
    fn firefox_key_differs_from_chrome() {
        type I = WindowsHostInstaller<crate::windows::paths::WindowsPaths>;
        assert_ne!(
            I::registry_key(Browser::Chrome),
            I::registry_key(Browser::Firefox)
        );
    }
}
