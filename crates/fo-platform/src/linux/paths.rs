//! Linux のデータ配置先。XDG Base Directory 仕様に従う。

use std::path::PathBuf;

use crate::{env_path, PlatformPaths};

const APP_DIR: &str = "file-origin";

pub struct LinuxPaths;

impl LinuxPaths {
    fn home(&self) -> PathBuf {
        env_path("HOME").unwrap_or_else(|| PathBuf::from("."))
    }
}

impl PlatformPaths for LinuxPaths {
    fn data_dir(&self) -> PathBuf {
        env_path("XDG_DATA_HOME")
            .unwrap_or_else(|| self.home().join(".local").join("share"))
            .join(APP_DIR)
    }

    fn config_dir(&self) -> PathBuf {
        env_path("XDG_CONFIG_HOME")
            .unwrap_or_else(|| self.home().join(".config"))
            .join(APP_DIR)
    }

    fn cache_dir(&self) -> PathBuf {
        env_path("XDG_CACHE_HOME")
            .unwrap_or_else(|| self.home().join(".cache"))
            .join(APP_DIR)
    }

    fn log_dir(&self) -> PathBuf {
        // XDG_STATE_HOME はログのような「再現できるが残したい」データの置き場所。
        env_path("XDG_STATE_HOME")
            .unwrap_or_else(|| self.home().join(".local").join("state"))
            .join(APP_DIR)
    }

    fn runtime_dir(&self) -> PathBuf {
        // Unix ドメインソケットを置く場所。XDG_RUNTIME_DIR は 0700 で
        // ログアウト時に消えるため、ソケットの置き場所として適切。
        // 無い環境（systemd 以外のセッション）では /tmp に落とすが、
        // その場合はソケットの権限を自前で 0600 にする必要がある。
        env_path("XDG_RUNTIME_DIR")
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_DIR)
    }

    fn default_download_dirs(&self) -> Vec<PathBuf> {
        // 本来は xdg-user-dirs の ~/.config/user-dirs.dirs を読んで
        // XDG_DOWNLOAD_DIR を引くべき（ロケールによって「ダウンロード」等になる）。
        // TODO(M2): user-dirs.dirs のパースを入れる。
        if let Some(dir) = env_path("XDG_DOWNLOAD_DIR") {
            return vec![dir];
        }
        let candidate = self.home().join("Downloads");
        if candidate.is_dir() {
            vec![candidate]
        } else {
            Vec::new()
        }
    }
}
