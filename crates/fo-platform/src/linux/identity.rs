//! Linux のファイル安定識別子 = `(st_dev, st_ino)`。
//!
//! Windows と違い FFI は要らない。std が `MetadataExt` で両方を stable に公開している。

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::{Error, FileIdentity, FileKey, Result, StableFileId, VolumeId};

pub struct LinuxIdentity;

impl FileIdentity for LinuxIdentity {
    fn stable_id(&self, path: &Path) -> Result<StableFileId> {
        // シンボリックリンクは辿る。追跡したいのは実体であってリンクではない。
        let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(path.to_path_buf()),
            std::io::ErrorKind::PermissionDenied => Error::PermissionDenied(path.to_path_buf()),
            _ => Error::Io(e),
        })?;

        Ok(StableFileId {
            volume: VolumeId(format!("{:016x}", meta.dev())),
            file: FileKey(format!("{:016x}", meta.ino())),
        })
    }

    fn resolve_path(&self, _id: &StableFileId) -> Result<Option<PathBuf>> {
        // Linux に inode → path の一般的な逆引きは存在しない。
        // ファイルシステム全体を走査すれば求まるが、それは逆引きではなく全探索。
        //
        // したがって File Origin は DB の file_paths 索引で解決し、
        // 見つからなければ再スキャンする（README §7.3）。
        // これは実装漏れではなく、OS の性質。
        Err(Error::Unsupported(
            "Linux に inode→path の逆引きは無い（DB 索引で代替する）",
        ))
    }

    fn supports_reverse_lookup(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_file_yields_same_id() {
        let id1 = LinuxIdentity.stable_id(Path::new("/etc/hostname"));
        let id2 = LinuxIdentity.stable_id(Path::new("/etc/hostname"));
        match (id1, id2) {
            (Ok(a), Ok(b)) => assert_eq!(a, b),
            // コンテナ等で /etc/hostname が無い場合はこのテストを飛ばす。
            _ => eprintln!("skip: /etc/hostname を読めない環境"),
        }
    }

    #[test]
    fn missing_file_is_not_found() {
        let err = LinuxIdentity
            .stable_id(Path::new("/nonexistent/fo-test/nothing"))
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }
}
