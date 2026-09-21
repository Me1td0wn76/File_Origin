//! Windows のデータ配置先。
//!
//! `%LOCALAPPDATA%` はローミングしない。DB はマシン固有の
//! ボリューム識別子・inode を含むため、プロファイル間で同期されると壊れる。
//! よって `%APPDATA%`（ローミング対象）には置かない。

use std::path::{Path, PathBuf};

use crate::{env_path, PlatformPaths};

/// `\\?\C:\foo` → `C:\foo`、`\\?\UNC\server\share` → `\\server\share`。
///
/// verbatim 形式は 260 文字超のパスを扱うための Win32 の仕組みで、
/// `canonicalize` が常にこれを返す。表示にも比較にも向かないので剥がす。
/// それ以外の形（既に剥がれている、デバイスパス等）はそのまま返す。
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

const APP_DIR: &str = "FileOrigin";

pub struct WindowsPaths;

impl WindowsPaths {
    fn local_app_data(&self) -> PathBuf {
        env_path("LOCALAPPDATA")
            .or_else(|| env_path("USERPROFILE").map(|p| p.join("AppData").join("Local")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR)
    }
}

impl PlatformPaths for WindowsPaths {
    fn data_dir(&self) -> PathBuf {
        self.local_app_data()
    }

    fn config_dir(&self) -> PathBuf {
        // 設定はローミングしてよい（マシン固有の値を含まないため）。
        env_path("APPDATA")
            .map(|p| p.join(APP_DIR))
            .unwrap_or_else(|| self.local_app_data())
    }

    fn cache_dir(&self) -> PathBuf {
        self.local_app_data().join("cache")
    }

    fn log_dir(&self) -> PathBuf {
        self.local_app_data().join("logs")
    }

    fn runtime_dir(&self) -> PathBuf {
        // Windows に XDG_RUNTIME_DIR 相当は無い。名前付きパイプは
        // ファイルシステム上のパスを使わないため、ここは一時領域でよい。
        env_path("TEMP")
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_DIR)
    }

    fn canonical(&self, path: &Path) -> std::io::Result<PathBuf> {
        let canon = std::fs::canonicalize(path)?;
        Ok(strip_verbatim_prefix(canon))
    }

    fn default_download_dirs(&self) -> Vec<PathBuf> {
        // 本来は SHGetKnownFolderPath(FOLDERID_Downloads) で引くべきで、
        // ユーザーが場所を変更していると %USERPROFILE%\Downloads は外れる。
        // TODO(M2): Known Folder API に置き換える。
        env_path("USERPROFILE")
            .map(|p| vec![p.join("Downloads")])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_drive_verbatim() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\Users\x\a.zip")),
            PathBuf::from(r"C:\Users\x\a.zip")
        );
    }

    #[test]
    fn strips_unc_verbatim() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\nas\share\a.zip")),
            PathBuf::from(r"\\nas\share\a.zip")
        );
    }

    #[test]
    fn leaves_plain_paths_alone() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"C:\plain\a.zip")),
            PathBuf::from(r"C:\plain\a.zip")
        );
    }
}
