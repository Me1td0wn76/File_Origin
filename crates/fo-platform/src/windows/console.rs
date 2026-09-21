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
//!
//! ## リダイレクトを壊さない
//!
//! `fo-daemon --foreground > log.txt` や `| head` のように出力先が
//! 指定されている場合、その handle は GUI サブシステムでも継承される。
//! 無条件に `CONOUT$` を差し込むと**リダイレクトを奪ってしまう**ので、
//! 既に有効な handle があるものは触らない。

use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
use windows_sys::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_HANDLE,
    STD_OUTPUT_HANDLE,
};

// Win32 定数。windows-sys のバージョン差でインポート名が揺れるため値を直接置く。
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const OPEN_EXISTING: u32 = 3;

/// その標準ハンドルが既に有効か（リダイレクト済みか）。
fn is_redirected(which: STD_HANDLE) -> bool {
    // SAFETY: 定数を渡すだけ。失敗しても副作用は無い。
    let h = unsafe { GetStdHandle(which) };
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

/// 起動元の端末に標準出力・標準エラーを繋ぐ。繋げたら `true`。
///
/// 既にリダイレクトされているものはそのまま残す。
/// どちらも既にリダイレクト済みなら、コンソールに繋ぐ必要がないので `true`。
pub fn attach_parent_console() -> bool {
    let out_redirected = is_redirected(STD_OUTPUT_HANDLE);
    let err_redirected = is_redirected(STD_ERROR_HANDLE);
    if out_redirected && err_redirected {
        // 出力先は指定済み。触らないのが正しい。
        return true;
    }

    // SAFETY: 引数は Win32 の規定どおりの定数。失敗しても副作用は無い。
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        // 端末から起動されていない。新しい窓は作らない。
        return false;
    }

    let Some(conout) = open_conout() else {
        return false;
    };

    // リダイレクトされていない側だけを差し替える。
    //
    // Rust の std は書き込みのたびに GetStdHandle を引くので、
    // SetStdHandle で差し替えれば `println!` / `eprintln!` がそのまま届く。
    let mut used = false;
    for (which, redirected) in [
        (STD_OUTPUT_HANDLE, out_redirected),
        (STD_ERROR_HANDLE, err_redirected),
    ] {
        if redirected {
            continue;
        }
        // SAFETY: conout は CreateFileW が返した有効なハンドル。
        if unsafe { SetStdHandle(which, conout) } != 0 {
            used = true;
        }
    }

    if !used {
        // どこにも使わなかったハンドルは閉じる。
        // SAFETY: conout は有効で、std には渡っていない。
        unsafe {
            let _ = CloseHandle(conout);
        }
        return false;
    }
    true
}

/// コンソールの出力側を開く。
fn open_conout() -> Option<HANDLE> {
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
        None
    } else {
        Some(handle)
    }
}
