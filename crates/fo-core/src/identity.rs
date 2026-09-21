//! 同一性判定のはしご。
//!
//! **File Origin で最も間違えやすい部分。** SHA-256 だけではファイルの移動先は分からない。
//! ハッシュは「同じ内容か」には答えるが、「どこへ行ったか」にも
//! 「コピーか移動か」にも答えない。
//!
//! そこで **OS の安定識別子を主、ハッシュを従** として組み合わせる。
//! 判定は上から順に評価し、最初に成立したものを採る（README §8.1）。
//!
//! この判定を誤ると入手元を取り違え、記録が信用できなくなる。
//! 純粋関数にしてあるのは、ここを徹底的にテストするため。

use std::path::{Path, PathBuf};

use fo_platform::StableFileId;

use crate::model::{Confidence, Digest, FileRecord};

/// スキャンで観測したファイル。
#[derive(Debug, Clone)]
pub struct Observed {
    pub stable_id: StableFileId,
    pub path: PathBuf,
    pub size: u64,
    /// 未計算なら `None`。遅延計算の途中でも判定は始められる。
    pub sha256: Option<Digest>,
}

/// 観測したファイルが、既知のどのレコードに対応するか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 同一ファイル。位置も内容も変わっていない。
    Same { file_id: i64 },
    /// 同一ファイルが別の場所に移った。入手元はそのまま引き継ぐ。
    Moved { file_id: i64, from: PathBuf },
    /// 同一ファイルの内容が更新された。識別子は同じ。
    /// 新しいダイジェストを版として追記する。
    Updated { file_id: i64 },
    /// 内容が同じ別ファイル＝コピー。
    /// 新しいレコードを作り、`derived_from` で親を指す。
    Copied { from_file_id: i64 },
    /// 既知のどれにも対応しない。新規に記録する。
    New,
}

impl Verdict {
    /// この判定をどれくらい信じてよいか。
    pub fn confidence(&self) -> Confidence {
        match self {
            // 安定識別子の一致は OS が保証する。
            Verdict::Same { .. } | Verdict::Updated { .. } => Confidence::Certain,
            // ハッシュ一致からの推定。偶然の一致はまず無いが、
            // 「移動」と「コピー」の区別は旧パスの生存という状況証拠に頼っている。
            Verdict::Moved { .. } | Verdict::Copied { .. } => Confidence::High,
            Verdict::New => Confidence::Certain,
        }
    }
}

/// 観測したファイルを既知のレコード群と突き合わせる。
///
/// `path_exists` は旧パスがまだ実在するかを答える述語。
/// 移動とコピーの区別はこれだけで決まる（旧パスが消えていれば移動、残っていればコピー）。
/// I/O を引数に切り出してあるのは、この関数を純粋に保ってテストするため。
pub fn classify<F>(observed: &Observed, known: &[FileRecord], path_exists: F) -> Verdict
where
    F: Fn(&Path) -> bool,
{
    // 1. 安定識別子が一致 → 同一ファイル確定。
    if let Some(rec) = known.iter().find(|r| r.stable_id == observed.stable_id) {
        return match (&rec.sha256, &observed.sha256) {
            // 4. 識別子は同じだが内容が変わった → 更新。
            (Some(known_hash), Some(seen_hash)) if known_hash != seen_hash => {
                Verdict::Updated { file_id: rec.id }
            }
            // ハッシュが片方でも未計算なら、内容の変化は判断できない。
            // 識別子が同じである以上「同じファイル」であることは確かなので Same とする。
            // 内容の差はハッシュが揃った時点で Updated として拾い直せる。
            _ => Verdict::Same { file_id: rec.id },
        };
    }

    // ここから先はハッシュが要る。未計算なら判定を保留して新規扱いにする。
    // 誤って既存レコードに結びつけるより、後で再判定する方が安全。
    let Some(seen_hash) = observed.sha256.as_ref() else {
        return Verdict::New;
    };

    // 2 / 3. 内容が一致する既知レコードを探す。
    // サイズを先に見るのは、ハッシュ衝突対策ではなく単なる枝刈り。
    let same_content = known
        .iter()
        .filter(|r| r.size == observed.size)
        .find(|r| r.sha256.as_ref() == Some(seen_hash));

    match same_content {
        Some(rec) => {
            if path_exists(&rec.current_path) {
                // 旧パスにも実体が残っている → コピー。
                Verdict::Copied {
                    from_file_id: rec.id,
                }
            } else {
                // 旧パスが消えている → 移動。別ボリュームへ移した場合もここに来る
                // （その場合は安定識別子が変わるため 1 では捕まらない）。
                Verdict::Moved {
                    file_id: rec.id,
                    from: rec.current_path.clone(),
                }
            }
        }
        None => Verdict::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FileStatus;
    use fo_platform::{FileKey, VolumeId};

    fn sid(vol: &str, key: &str) -> StableFileId {
        StableFileId {
            volume: VolumeId(vol.into()),
            file: FileKey(key.into()),
        }
    }

    fn record(
        id: i64,
        vol: &str,
        key: &str,
        path: &str,
        size: u64,
        hash: Option<&str>,
    ) -> FileRecord {
        FileRecord {
            id,
            stable_id: sid(vol, key),
            current_path: PathBuf::from(path),
            size,
            sha256: hash.map(|h| Digest(h.into())),
            mtime: 0,
            status: FileStatus::Present,
            derived_from: None,
        }
    }

    fn observed(vol: &str, key: &str, path: &str, size: u64, hash: Option<&str>) -> Observed {
        Observed {
            stable_id: sid(vol, key),
            path: PathBuf::from(path),
            size,
            sha256: hash.map(|h| Digest(h.into())),
        }
    }

    const NOTHING_EXISTS: fn(&Path) -> bool = |_| false;
    const EVERYTHING_EXISTS: fn(&Path) -> bool = |_| true;

    #[test]
    fn stable_id_match_is_same_file() {
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        // 名前もパスも変わったが、識別子が同じなら同一ファイル。
        let obs = observed("v1", "f1", "/b/renamed.zip", 100, Some("aa"));
        assert_eq!(
            classify(&obs, &known, NOTHING_EXISTS),
            Verdict::Same { file_id: 1 }
        );
    }

    #[test]
    fn same_id_different_hash_is_update() {
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        let obs = observed("v1", "f1", "/a/x.zip", 120, Some("bb"));
        assert_eq!(
            classify(&obs, &known, EVERYTHING_EXISTS),
            Verdict::Updated { file_id: 1 }
        );
    }

    #[test]
    fn same_hash_and_old_path_gone_is_move() {
        // 別ボリュームへ移動すると識別子が変わる。ハッシュで拾い直す。
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        let obs = observed("v2", "f9", "/mnt/usb/x.zip", 100, Some("aa"));
        assert_eq!(
            classify(&obs, &known, NOTHING_EXISTS),
            Verdict::Moved {
                file_id: 1,
                from: PathBuf::from("/a/x.zip")
            }
        );
    }

    #[test]
    fn same_hash_and_old_path_alive_is_copy() {
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        let obs = observed("v1", "f2", "/a/copy.zip", 100, Some("aa"));
        assert_eq!(
            classify(&obs, &known, EVERYTHING_EXISTS),
            Verdict::Copied { from_file_id: 1 }
        );
    }

    #[test]
    fn unhashed_new_file_is_not_guessed() {
        // ハッシュ未計算のまま既存レコードに結びつけない。
        // 誤って結びつけるより、後で再判定する方が安全。
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        let obs = observed("v1", "f2", "/a/other.zip", 100, None);
        assert_eq!(classify(&obs, &known, EVERYTHING_EXISTS), Verdict::New);
    }

    #[test]
    fn unhashed_known_file_is_still_same() {
        // 識別子が一致するなら、ハッシュが無くても同一ファイルと言える。
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, None)];
        let obs = observed("v1", "f1", "/a/x.zip", 100, Some("aa"));
        assert_eq!(
            classify(&obs, &known, EVERYTHING_EXISTS),
            Verdict::Same { file_id: 1 }
        );
    }

    #[test]
    fn different_size_same_hash_is_not_matched() {
        // 現実には起きないが、サイズ枝刈りが効いていることの確認。
        let known = vec![record(1, "v1", "f1", "/a/x.zip", 100, Some("aa"))];
        let obs = observed("v1", "f2", "/a/y.zip", 999, Some("aa"));
        assert_eq!(classify(&obs, &known, NOTHING_EXISTS), Verdict::New);
    }

    #[test]
    fn empty_known_set_is_new() {
        let obs = observed("v1", "f1", "/a/x.zip", 100, Some("aa"));
        assert_eq!(classify(&obs, &[], NOTHING_EXISTS), Verdict::New);
    }
}
