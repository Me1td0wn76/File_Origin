//! Linux の Native Messaging ホスト登録。
//!
//! Windows と違いレジストリが無く、**決まった場所に JSON を置くだけ**。
//! そのぶん単純だが、ブラウザごとにディレクトリが違う。
//!
//! すべて `$HOME` 配下なので管理者権限は要らない。

use std::path::PathBuf;

use crate::nativehost::{Browser, HostManifest, Installed, NativeHostInstaller, HOST_NAME};
use crate::{env_path, PlatformPaths, Result};

pub struct LinuxHostInstaller<P> {
    _paths: P,
}

impl<P: PlatformPaths> LinuxHostInstaller<P> {
    pub fn new(paths: P) -> Self {
        Self { _paths: paths }
    }
}

fn home() -> PathBuf {
    env_path("HOME").unwrap_or_else(|| PathBuf::from("."))
}

/// ブラウザごとのマニフェスト置き場。
///
/// Chrome 系は `$XDG_CONFIG_HOME` ではなく `~/.config/<product>/` 固定。
/// ブラウザ側がそう決め打ちしているので、XDG に寄せてはいけない。
fn host_dir(browser: Browser) -> PathBuf {
    let h = home();
    match browser {
        Browser::Chrome => h.join(".config/google-chrome/NativeMessagingHosts"),
        Browser::Chromium => h.join(".config/chromium/NativeMessagingHosts"),
        // Linux の Edge も Chromium 系の規約に従う。
        Browser::Edge => h.join(".config/microsoft-edge/NativeMessagingHosts"),
        Browser::Firefox => h.join(".mozilla/native-messaging-hosts"),
    }
}

impl<P: PlatformPaths> NativeHostInstaller for LinuxHostInstaller<P> {
    fn manifest_path(&self, browser: Browser) -> PathBuf {
        host_dir(browser).join(format!("{HOST_NAME}.json"))
    }

    fn install(&self, browser: Browser, manifest: &HostManifest) -> Result<Installed> {
        let path = self.manifest_path(browser);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, manifest.to_json(browser))?;
        Ok(Installed {
            browser,
            manifest_path: path,
            registry_key: None,
        })
    }

    fn uninstall(&self, browser: Browser) -> Result<()> {
        let path = self.manifest_path(browser);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn is_installed(&self, browser: Browser) -> bool {
        self.manifest_path(browser).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_browser_has_its_own_directory() {
        let dirs: Vec<PathBuf> = Browser::all().iter().map(|b| host_dir(*b)).collect();
        let mut uniq = dirs.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(dirs.len(), uniq.len(), "ブラウザごとに別ディレクトリ");
    }

    #[test]
    fn firefox_uses_mozilla_convention() {
        let p = host_dir(Browser::Firefox);
        assert!(p.ends_with("native-messaging-hosts"), "{p:?}");
    }

    #[test]
    fn chrome_uses_config_dir_not_xdg() {
        // Chrome は ~/.config/google-chrome/ 固定。XDG_CONFIG_HOME を見ない。
        let p = host_dir(Browser::Chrome);
        assert!(
            p.to_string_lossy().contains(".config/google-chrome"),
            "{p:?}"
        );
    }
}
