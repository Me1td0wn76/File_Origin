//! Windows の IPC エンドポイント名。
//!
//! 名前付きパイプ `\.\pipeile-origin`。ファイルシステム上に実体を持たないので、
//! 置き場所（XDG_RUNTIME_DIR 相当）を考える必要がない。
//!
//! TODO(M6): 現在のユーザー SID のみを許可する DACL を設定する（README §11）。
//! 既定の名前付きパイプはローカルの他ユーザーからも開けうる。

use crate::ipc::{IpcName, LocalSocket};

pub fn transport() -> LocalSocket {
    LocalSocket::new(IpcName::Namespaced("file-origin".to_string()))
}
