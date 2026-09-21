//! Linux 実装。
//!
//! 対応表は README §7.2 を参照。

mod identity;
mod ipc;
mod nativehost;
mod origin;
pub(crate) mod paths;

use crate::ipc::LocalSocket;
use crate::nativehost::NativeHostInstaller;
use crate::watcher::NotifyWatcher;
use crate::{
    Capabilities, Capability, FileIdentity, FsWatcher, IpcTransport, OriginMetadata, Platform,
    PlatformPaths, Result,
};

pub struct LinuxPlatform {
    identity: identity::LinuxIdentity,
    origin: origin::LinuxOriginMetadata,
    paths: paths::LinuxPaths,
    ipc: LocalSocket,
    host_installer: nativehost::LinuxHostInstaller<paths::LinuxPaths>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            identity: identity::LinuxIdentity,
            origin: origin::LinuxOriginMetadata::new(),
            paths: paths::LinuxPaths,
            host_installer: nativehost::LinuxHostInstaller::new(paths::LinuxPaths),
            ipc: ipc::transport(&paths::LinuxPaths),
        }
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for LinuxPlatform {
    fn identity(&self) -> &dyn FileIdentity {
        &self.identity
    }

    fn origin_meta(&self) -> &dyn OriginMetadata {
        &self.origin
    }

    fn paths(&self) -> &dyn PlatformPaths {
        &self.paths
    }

    fn ipc(&self) -> &dyn IpcTransport {
        &self.ipc
    }

    fn host_installer(&self) -> &dyn NativeHostInstaller {
        &self.host_installer
    }

    fn new_watcher(&self) -> Result<Box<dyn FsWatcher>> {
        Ok(Box::new(NotifyWatcher::new()?))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform_name: format!("Linux ({})", std::env::consts::ARCH),
            stable_file_id: Capability::Available,
            reverse_lookup: Capability::Unavailable {
                // Windows の「未実装」と違い、こちらは OS の性質。
                // 理由を書き分けることで、将来実装すれば直る話ではないと伝わる。
                why: "OS に inode→path の逆引きが無い。DB 索引で代替する",
            },
            origin_sources: self.origin.available_sources(),
            change_journal: Capability::NeedsPrivilege {
                // ADR-0005: Windows の USN と同格の「任意の高速化」。
                how: "fanotify は CAP_SYS_ADMIN が必要",
            },
            // xattr は「読める」が「入っている」とは限らない。
            // これを説明しないと、ユーザーは実装が壊れていると考える。
            advice: vec![
                "Linux では主要ブラウザが xattr を書きません（wget/curl --xattr のみ）",
                "→ ブラウザ拡張の導入を強く推奨します",
            ],
        }
    }
}
