//! Windows のコンソール接続。
//!
//! `windows_subsystem = "windows"` を付けたプロセスは**コンソールを持たない**。
//! 常駐サービスとしてはそれが正しいが、端末から `--foreground` で起動したときに
//! 何も出なくなると、動いているのかどうか分からない。
//!
//! そこで、起動元の端末があるならそこへ繋ぎ直す。
//! 端末から起動されていなければ（エクスプローラからのダブルクリック、
//! サービスからの起動など）何もしない。**新しいコンソールは作らない** —
//! 黙って窓を開くのは、この Issue で消そうとしているものそのもの。

use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
use windows_sys::Win32::System::Console::{
    AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};

// Win32 定数。windows-sys のバージョン差でインポート名が揺れるため値を直接置く。
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const OPEN_EXISTING: u32 = 3;

/// 起動元の端末に標準出力・標準エラーを繋ぐ。繋げたら `true`。
pub fn attach_parent_console() -> bool {
    // SAFETY: 引数は Win32 の規定どおりの定数。失敗しても副作用は無い。
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
    if !attached {
        return false;
    }

    // AttachConsole しただけでは、このプロセスの標準ハンドルは無効なまま。
    // コンソールの出力側（CONOUT$）を開いて差し替える。
    //
    // Rust の std は書き込みのたびに GetStdHandle を引くので、
    // SetStdHandle で差し替えれば `println!` / `eprintln!` がそのまま届く。
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();

    // SAFETY: name は null 終端済み。他の引数は Win32 の規定どおり。
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_WRITE | FILE_SHARE_READ,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }

    // SAFETY: handle は CreateFileW が返した有効なハンドル。
    let ok = unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle) != 0 && SetStdHandle(STD_ERROR_HANDLE, handle) != 0
    };

    if !ok {
        // 差し替えに失敗したら閉じる。開きっぱなしにしても使い道がない。
        // SAFETY: handle は有効で、まだ std に渡っていない。
        unsafe {
            let _ = CloseHandle(handle);
        }
        return false;
    }
    true
}
