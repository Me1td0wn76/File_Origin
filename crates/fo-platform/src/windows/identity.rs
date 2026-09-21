//! Windows のファイル安定識別子。
//!
//! `GetFileInformationByHandleEx` に `FileIdInfo` を要求して
//! `(VolumeSerialNumber, FileId128)` を得る。これはファイルが移動・リネームされても
//! 同一ボリューム内であれば変わらない。
//!
//! std の `MetadataExt::file_index()` は nightly 限定（`windows_by_handle`）なので
//! stable では FFI が要る。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandleEx, FILE_ID_INFO,
};

use crate::{Error, FileIdentity, FileKey, Result, StableFileId, VolumeId};

// Win32 定数。windows-sys のバージョン差でインポート名が揺れるため、
// 値を直接置いている（値自体は Windows SDK で固定）。
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const FILE_SHARE_ALL: u32 = 0x0000_0007; // READ | WRITE | DELETE
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000; // ディレクトリを開くのに要る
const FILE_INFO_CLASS_FILE_ID_INFO: i32 = 18; // FILE_INFO_BY_HANDLE_CLASS::FileIdInfo

pub struct WindowsIdentity;

impl FileIdentity for WindowsIdentity {
    fn stable_id(&self, path: &Path) -> Result<StableFileId> {
        let info = file_id_info(path)?;

        // FileId128 は 16 バイト。バイト列を素直に 16 進で文字列化する。
        // 数値として解釈しないのは、上位/下位の並びに依存したくないため。
        let file_hex: String = info
            .FileId
            .Identifier
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        Ok(StableFileId {
            volume: VolumeId(format!("{:016x}", info.VolumeSerialNumber)),
            file: FileKey(file_hex),
        })
    }

    fn resolve_path(&self, _id: &StableFileId) -> Result<Option<PathBuf>> {
        // Windows は `OpenFileById` で OS ネイティブに逆引きできる。
        // ただしボリュームのルートハンドルが要るので、M3 の監視実装と合わせて入れる。
        // それまでは DB の索引で解決する（Linux と同じ経路）。
        Err(Error::Unsupported("OpenFileById は未実装 (M3 で対応)"))
    }

    fn supports_reverse_lookup(&self) -> bool {
        // OS は対応しているが、まだ実装していない。
        // 「OS ができるか」ではなく「いま使えるか」を返す。
        // 嘘をつくと呼び出し側がフォールバックを用意しなくなる。
        false
    }
}

fn file_id_info(path: &Path) -> Result<FILE_ID_INFO> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: wide は null 終端済み。他の引数は Win32 の規定どおりの定数と null。
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_ALL,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(map_last_error(path));
    }

    let mut info = FILE_ID_INFO {
        VolumeSerialNumber: 0,
        FileId: windows_sys::Win32::Storage::FileSystem::FILE_ID_128 { Identifier: [0; 16] },
    };

    // SAFETY: handle は有効。info は FILE_ID_INFO のサイズちょうどの領域。
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_INFO_CLASS_FILE_ID_INFO,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };

    // SAFETY: handle は CreateFileW が返した有効なハンドル。
    // 戻り値は握りつぶす。ここで閉じられなくても呼び出し側にできることは無い。
    unsafe {
        let _ = CloseHandle(handle);
    }

    if ok == 0 {
        // FAT32 / exFAT やネットワーク共有では FileIdInfo が取れないことがある。
        // 呼び出し側は SHA-256 による同定にフォールバックする。
        return Err(Error::Unsupported(
            "このファイルシステムは FileIdInfo に未対応（FAT32 / 一部のネットワーク共有など）",
        ));
    }

    Ok(info)
}

fn map_last_error(path: &Path) -> Error {
    let io = std::io::Error::last_os_error();
    match io.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(path.to_path_buf()),
        std::io::ErrorKind::PermissionDenied => Error::PermissionDenied(path.to_path_buf()),
        _ => Error::Io(io),
    }
}
