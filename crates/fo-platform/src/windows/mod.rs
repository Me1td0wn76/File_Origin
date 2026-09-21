//! Windows 実装。
//!
//! 対応表は README §7.2 を参照。

mod identity;
mod origin;
mod paths;

use crate::{Capabilities, Capability, FileIdentity, OriginMetadata, Platform, PlatformPaths};

pub struct WindowsPlatform {
    identity: identity::WindowsIdentity,
    origin: origin::WindowsOriginMetadata,
    paths: paths::WindowsPaths,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self {
            identity: identity::WindowsIdentity,
            origin: origin::WindowsOriginMetadata,
            paths: paths::WindowsPaths,
        }
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WindowsPlatform {
    fn identity(&self) -> &dyn FileIdentity {
        &self.identity
    }

    fn origin_meta(&self) -> &dyn OriginMetadata {
        &self.origin
    }

    fn paths(&self) -> &dyn PlatformPaths {
        &self.paths
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform_name: format!("Windows ({})", std::env::consts::ARCH),
            stable_file_id: Capability::Available,
            reverse_lookup: Capability::Unavailable {
                // OS は OpenFileById で対応しているが、こちらが未実装。
                // Linux の「原理的に不可」とは理由が違うので、そう書く。
                why: "OS は OpenFileById に対応。実装が M3 待ち",
            },
            origin_sources: self.origin.available_sources(),
            change_journal: Capability::NeedsPrivilege {
                // ADR-0005: 特権が要る機能は任意の高速化として扱う。
                // 既定は差分スキャンで動くので、無くても機能は失わない。
                how: "USN Change Journal は管理者権限が必要（保持は約 1 週間）",
            },
            // Zone.Identifier は主要ブラウザが自動で書くため、
            // Linux と違い OS メタデータ経路が実用になる。特記は不要。
            advice: Vec::new(),
        }
    }
}
