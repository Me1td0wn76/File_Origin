//! テスト用のインメモリ実装。
//!
//! **これが `fo-core` のテストを OS から切り離す仕掛け。**
//! Windows の開発機で Linux 相当の挙動も検証できるのは、ここがあるから。
//!
//! モックは「能力がある環境」だけでなく **「能力がない環境」も再現できる**ようにしてある。
//! フォールバック経路はテストされにくく、壊れても気づかないため。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ipc::{IpcName, LocalSocket};
use crate::watcher::NotifyWatcher;
use crate::{
    Capabilities, Capability, Error, FileIdentity, FileKey, FsWatcher, IpcTransport,
    OriginMetadata, OsOrigin, OsOriginSource, Platform, PlatformPaths, Result, StableFileId,
    VolumeId,
};

/// モックの振る舞いを決める設定。
#[derive(Debug, Clone)]
pub struct MockConfig {
    /// 逆引きができる環境として振る舞うか（Windows 相当 / Linux 相当の切り替え）。
    pub reverse_lookup: bool,
    /// 入手元メタデータを読めるか。`false` なら Unsupported を返す。
    pub origin_readable: bool,
    /// 変更ジャーナルが使えるか。
    pub journal: Capability,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            reverse_lookup: false,
            origin_readable: true,
            journal: Capability::NeedsPrivilege { how: "mock" },
        }
    }
}

#[derive(Default)]
struct Inner {
    ids: HashMap<PathBuf, StableFileId>,
    origins: HashMap<PathBuf, Vec<OsOrigin>>,
}

pub struct MockPlatform {
    config: MockConfig,
    inner: Mutex<Inner>,
    paths: MockPaths,
    ipc: LocalSocket,
}

impl MockPlatform {
    pub fn new() -> Self {
        Self::with_config(MockConfig::default())
    }

    pub fn with_config(config: MockConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(Inner::default()),
            paths: MockPaths(std::env::temp_dir().join("file-origin-test")),
            // テストが実際に IPC を張ることは無いが、trait を満たすために要る。
            ipc: LocalSocket::new(IpcName::Namespaced("file-origin-mock".to_string())),
        }
    }

    /// テスト用にファイルを登録する。実ファイルは不要。
    pub fn add_file(&self, path: impl Into<PathBuf>, volume: &str, key: &str) {
        let id = StableFileId {
            volume: VolumeId(volume.to_string()),
            file: FileKey(key.to_string()),
        };
        self.inner.lock().unwrap().ids.insert(path.into(), id);
    }

    /// テスト用に入手元を登録する。
    pub fn add_origin(&self, path: impl Into<PathBuf>, origin: OsOrigin) {
        self.inner
            .lock()
            .unwrap()
            .origins
            .entry(path.into())
            .or_default()
            .push(origin);
    }

    /// ファイルが移動したことにする。安定識別子は維持される。
    pub fn move_file(&self, from: &Path, to: impl Into<PathBuf>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(id) = inner.ids.remove(from) {
            inner.ids.insert(to.into(), id);
        }
    }
}

impl Default for MockPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FileIdentity for MockPlatform {
    fn stable_id(&self, path: &Path) -> Result<StableFileId> {
        self.inner
            .lock()
            .unwrap()
            .ids
            .get(path)
            .cloned()
            .ok_or_else(|| Error::NotFound(path.to_path_buf()))
    }

    fn resolve_path(&self, id: &StableFileId) -> Result<Option<PathBuf>> {
        if !self.config.reverse_lookup {
            return Err(Error::Unsupported("mock: 逆引き無効"));
        }
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .ids
            .iter()
            .find(|(_, v)| *v == id)
            .map(|(k, _)| k.clone()))
    }

    fn supports_reverse_lookup(&self) -> bool {
        self.config.reverse_lookup
    }
}

impl OriginMetadata for MockPlatform {
    fn read_origin(&self, path: &Path) -> Result<Vec<OsOrigin>> {
        if !self.config.origin_readable {
            return Err(Error::Unsupported("mock: 入手元メタデータ無効"));
        }
        Ok(self
            .inner
            .lock()
            .unwrap()
            .origins
            .get(path)
            .cloned()
            .unwrap_or_default())
    }

    fn available_sources(&self) -> Vec<(OsOriginSource, Capability)> {
        let cap = if self.config.origin_readable {
            Capability::Available
        } else {
            Capability::Unavailable { why: "mock" }
        };
        vec![(OsOriginSource::Xattr, cap)]
    }

    fn write_origin(&self, _path: &Path, _origin: &OsOrigin) -> Result<()> {
        Err(Error::Unsupported("mock: 書き戻し未対応"))
    }
}

pub struct MockPaths(PathBuf);

impl PlatformPaths for MockPaths {
    fn data_dir(&self) -> PathBuf {
        self.0.join("data")
    }
    fn config_dir(&self) -> PathBuf {
        self.0.join("config")
    }
    fn cache_dir(&self) -> PathBuf {
        self.0.join("cache")
    }
    fn log_dir(&self) -> PathBuf {
        self.0.join("logs")
    }
    fn runtime_dir(&self) -> PathBuf {
        self.0.join("run")
    }
    fn default_download_dirs(&self) -> Vec<PathBuf> {
        vec![self.0.join("Downloads")]
    }
}

impl Platform for MockPlatform {
    fn identity(&self) -> &dyn FileIdentity {
        self
    }

    fn origin_meta(&self) -> &dyn OriginMetadata {
        self
    }

    fn paths(&self) -> &dyn PlatformPaths {
        &self.paths
    }

    fn ipc(&self) -> &dyn IpcTransport {
        &self.ipc
    }

    fn new_watcher(&self) -> Result<Box<dyn FsWatcher>> {
        Ok(Box::new(NotifyWatcher::new()?))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform_name: "Mock".to_string(),
            stable_file_id: Capability::Available,
            reverse_lookup: if self.config.reverse_lookup {
                Capability::Available
            } else {
                Capability::Unavailable {
                    why: "mock: 無効"
                }
            },
            origin_sources: self.available_sources(),
            change_journal: self.config.journal.clone(),
            advice: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_preserves_stable_id() {
        let p = MockPlatform::new();
        p.add_file("/a/setup.zip", "vol1", "ino1");

        let before = p.stable_id(Path::new("/a/setup.zip")).unwrap();
        p.move_file(Path::new("/a/setup.zip"), "/b/renamed.zip");
        let after = p.stable_id(Path::new("/b/renamed.zip")).unwrap();

        // 移動しても識別子は変わらない。これが移動追跡の土台。
        assert_eq!(before, after);
    }

    #[test]
    fn reverse_lookup_can_be_disabled() {
        let p = MockPlatform::with_config(MockConfig {
            reverse_lookup: false,
            ..Default::default()
        });
        p.add_file("/a/x", "vol1", "ino1");
        let id = p.stable_id(Path::new("/a/x")).unwrap();

        // Linux 相当の環境を再現できることの確認。
        assert!(matches!(p.resolve_path(&id), Err(Error::Unsupported(_))));
    }
}
