//! Linux の IPC エンドポイント名。
//!
//! `$XDG_RUNTIME_DIR/file-origin/file-origin.sock`。
//! 抽象名前空間ソケット（`@file-origin`）を使わないのは、**ファイル権限を持たない**ため。
//! `$XDG_RUNTIME_DIR` は 0700 なので、そこに置けば同一ユーザーに限定できる（README §11）。

use crate::ipc::{IpcName, LocalSocket};
use crate::PlatformPaths;

pub fn transport(paths: &dyn PlatformPaths) -> LocalSocket {
    LocalSocket::new(IpcName::FilePath(
        paths.runtime_dir().join("file-origin.sock"),
    ))
}
