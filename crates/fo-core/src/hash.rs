//! SHA-256 の計算。
//!
//! ADR-0007: **常にファイル全体を計算する。** 先頭 N MB の部分ハッシュは
//! 同一性判定に使えない（同じヘッダを持つ書庫や動画は普通に存在する）。
//! 誤って同一と判定すると入手元を取り違え、記録が信用できなくなる。
//!
//! 代わりに「記録」と「計算」を切り離す。ダウンロード直後は
//! 安定識別子と入手元だけを記録し、ハッシュは後から埋める。

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest as _, Sha256};

use crate::model::Digest;

/// 一度に読むサイズ。大きすぎるとメモリを食い、小さすぎると syscall が増える。
const CHUNK: usize = 64 * 1024;

/// ファイル全体の SHA-256 を計算する。
///
/// 数 GB のファイルでは時間がかかる。**呼び出し側は必ず非同期の文脈から呼ぶこと** —
/// ダウンロード直後の記録をこれで待たせてはいけない。
pub fn sha256_file(path: &Path) -> io::Result<Digest> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(Digest(hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hashes_known_input() {
        let dir = std::env::temp_dir().join("fo-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abc.txt");
        File::create(&path).unwrap().write_all(b"abc").unwrap();

        // "abc" の SHA-256 は広く知られた値。実装の取り違えをここで検出する。
        assert_eq!(
            sha256_file(&path).unwrap().as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hashes_empty_file() {
        let dir = std::env::temp_dir().join("fo-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.bin");
        File::create(&path).unwrap();

        assert_eq!(
            sha256_file(&path).unwrap().as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let _ = std::fs::remove_file(&path);
    }
}
