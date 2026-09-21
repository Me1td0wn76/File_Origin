//! SQLite 永続化層。
//!
//! **書き込み口はデーモン 1 つに集約する。** Native Messaging ホストは
//! ブラウザが複数プロファイルから同時に spawn するため、そこから直接 DB を
//! 触ると書き込みが競合する（README §9.1）。
//!
//! DB は暗号化しない（ADR-0006）。OS のファイル権限に依存する。

use std::path::{Path, PathBuf};

use fo_core::model::{Confidence, Digest, FileRecord, FileStatus, Origin, OriginSource};
use fo_platform::{FileKey, StableFileId, VolumeId};
use rusqlite::{params, Connection, OptionalExtension};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("データベース: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("データディレクトリを作成できない {path}: {source}")]
    DataDir { path: PathBuf, source: std::io::Error },

    #[error("DB に想定外の値が入っている: {0}")]
    Corrupt(String),
}

/// マイグレーション。追加するときは末尾に足し、既存を書き換えない。
/// 既に適用済みの DB を壊さないため。
const MIGRATIONS: &[(&str, &str)] = &[("0001_init", include_str!("../migrations/0001_init.sql"))];

pub struct Store {
    conn: Connection,
}

impl Store {
    /// DB を開き、必要ならマイグレーションを適用する。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::DataDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// メモリ上の DB を開く。テスト用。
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // WAL は読み書きの同時実行のため。デーモンが書いている最中に
        // CLI や GUI が読めないと使い物にならない。
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // 外部キーは既定で無効。ON DELETE CASCADE を効かせるために明示的に有効化する。
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 name       TEXT PRIMARY KEY,
                 applied_at INTEGER NOT NULL
             );",
        )?;

        for (name, sql) in MIGRATIONS {
            let applied: Option<i64> = self
                .conn
                .query_row(
                    "SELECT 1 FROM schema_migrations WHERE name = ?1",
                    params![name],
                    |r| r.get(0),
                )
                .optional()?;

            if applied.is_none() {
                self.conn.execute_batch(sql)?;
                self.conn.execute(
                    "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                    params![name, now()],
                )?;
            }
        }
        Ok(())
    }

    /// 安定識別子でファイルを引く。同一性判定の 1 段目。
    pub fn find_by_stable_id(&self, id: &StableFileId) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT f.id, f.volume_id, f.file_key, f.size, f.sha256, f.mtime,
                        f.status, f.derived_from,
                        COALESCE((SELECT p.path FROM file_paths p
                                  WHERE p.file_id = f.id AND p.is_current = 1
                                  LIMIT 1), '')
                 FROM files f
                 WHERE f.volume_id = ?1 AND f.file_key = ?2",
                params![id.volume.0, id.file.0],
                row_to_file,
            )
            .optional()?
            .transpose()
    }

    /// SHA-256 でファイルを引く。同一性判定の 2〜3 段目（移動 / コピーの判別）。
    pub fn find_by_sha256(&self, digest: &Digest) -> Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.volume_id, f.file_key, f.size, f.sha256, f.mtime,
                    f.status, f.derived_from,
                    COALESCE((SELECT p.path FROM file_paths p
                              WHERE p.file_id = f.id AND p.is_current = 1
                              LIMIT 1), '')
             FROM files f
             WHERE f.sha256 = ?1",
        )?;
        let rows = stmt.query_map(params![digest.as_str()], row_to_file)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().collect()
    }

    /// 新しいファイルを記録し、現在のパスを 1 件登録する。
    pub fn insert_file(
        &self,
        id: &StableFileId,
        path: &Path,
        size: u64,
        sha256: Option<&Digest>,
        mtime: i64,
    ) -> Result<i64> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO files
                 (volume_id, file_key, size, sha256, mtime, status,
                  first_seen_at, last_verified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'present', ?6, ?6)",
            params![id.volume.0, id.file.0, size as i64, sha256.map(|d| d.as_str()), mtime, ts],
        )?;
        let file_id = self.conn.last_insert_rowid();
        self.record_path(file_id, path)?;
        Ok(file_id)
    }

    /// 現在のパスを記録する。以前のパスは履歴として残す（消さない）。
    pub fn record_path(&self, file_id: i64, path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE file_paths SET is_current = 0 WHERE file_id = ?1 AND is_current = 1",
            params![file_id],
        )?;
        self.conn.execute(
            "INSERT INTO file_paths (file_id, path, is_current, observed_at)
             VALUES (?1, ?2, 1, ?3)",
            params![file_id, path.to_string_lossy(), now()],
        )?;
        Ok(())
    }

    /// 入手元を 1 件積む。既存の行は上書きしない。
    pub fn add_origin(&self, file_id: i64, origin: &Origin) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO origins
                 (file_id, url, referrer_url, host, acquired_at,
                  source, confidence, browser, profile, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                file_id,
                origin.url,
                origin.referrer_url,
                origin.host,
                origin.acquired_at,
                origin.source.as_str(),
                origin.confidence.as_str(),
                origin.browser,
                origin.profile,
                now(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// あるファイルの入手元を、確度の高い順に返す。
    pub fn origins_of(&self, file_id: i64) -> Result<Vec<Origin>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, referrer_url, host, acquired_at, source, confidence, browser, profile
             FROM origins WHERE file_id = ?1
             ORDER BY CASE confidence
                          WHEN 'certain' THEN 0 WHEN 'high' THEN 1
                          WHEN 'medium'  THEN 2 ELSE 3 END,
                      recorded_at DESC",
        )?;
        let rows = stmt.query_map(params![file_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (url, referrer_url, host, acquired_at, source, confidence, browser, profile) = row?;
            out.push(Origin {
                url,
                referrer_url,
                host,
                acquired_at,
                source: parse_source(&source)?,
                confidence: parse_confidence(&confidence)?,
                browser,
                profile,
            });
        }
        Ok(out)
    }

    /// パスからファイルを引く。現在のパスのみを見る。
    pub fn find_by_path(&self, path: &Path) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT f.id, f.volume_id, f.file_key, f.size, f.sha256, f.mtime,
                        f.status, f.derived_from, p.path
                 FROM files f
                 JOIN file_paths p ON p.file_id = f.id AND p.is_current = 1
                 WHERE p.path = ?1",
                params![path.to_string_lossy()],
                row_to_file,
            )
            .optional()?
            .transpose()
    }

    pub fn count_files(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?)
    }
}

type RowResult = rusqlite::Result<Result<FileRecord>>;

fn row_to_file(row: &rusqlite::Row<'_>) -> RowResult {
    let status_raw: String = row.get(6)?;
    let status = match status_raw.as_str() {
        "present" => FileStatus::Present,
        "missing" => FileStatus::Missing,
        "deleted" => FileStatus::Deleted,
        other => return Ok(Err(Error::Corrupt(format!("status='{other}'")))),
    };

    Ok(Ok(FileRecord {
        id: row.get(0)?,
        stable_id: StableFileId {
            volume: VolumeId(row.get(1)?),
            file: FileKey(row.get(2)?),
        },
        size: row.get::<_, i64>(3)? as u64,
        sha256: row.get::<_, Option<String>>(4)?.map(Digest),
        mtime: row.get(5)?,
        status,
        derived_from: row.get(7)?,
        current_path: PathBuf::from(row.get::<_, String>(8)?),
    }))
}

fn parse_source(s: &str) -> Result<OriginSource> {
    Ok(match s {
        "browser_ext" => OriginSource::BrowserExt,
        "zone_identifier" => OriginSource::ZoneIdentifier,
        "xattr" => OriginSource::Xattr,
        "gvfs" => OriginSource::Gvfs,
        "manual" => OriginSource::Manual,
        "history_db" => OriginSource::HistoryDb,
        other => return Err(Error::Corrupt(format!("source='{other}'"))),
    })
}

fn parse_confidence(s: &str) -> Result<Confidence> {
    Ok(match s {
        "certain" => Confidence::Certain,
        "high" => Confidence::High,
        "medium" => Confidence::Medium,
        "low" => Confidence::Low,
        other => return Err(Error::Corrupt(format!("confidence='{other}'"))),
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(v: &str, f: &str) -> StableFileId {
        StableFileId {
            volume: VolumeId(v.into()),
            file: FileKey(f.into()),
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let store = Store::open_in_memory().unwrap();
        // 2 回目の migrate() で落ちないこと。
        store.migrate().unwrap();
        assert_eq!(store.count_files().unwrap(), 0);
    }

    #[test]
    fn records_and_reads_back_a_file() {
        let store = Store::open_in_memory().unwrap();
        let id = sid("vol1", "key1");
        let file_id = store
            .insert_file(&id, Path::new("/dl/setup.zip"), 1234, Some(&Digest("aa".into())), 99)
            .unwrap();

        let found = store.find_by_stable_id(&id).unwrap().expect("見つかるはず");
        assert_eq!(found.id, file_id);
        assert_eq!(found.current_path, PathBuf::from("/dl/setup.zip"));
        assert_eq!(found.size, 1234);
    }

    #[test]
    fn path_history_is_kept_on_move() {
        let store = Store::open_in_memory().unwrap();
        let id = sid("vol1", "key1");
        let file_id = store
            .insert_file(&id, Path::new("/dl/setup.zip"), 10, None, 0)
            .unwrap();

        store.record_path(file_id, Path::new("/apps/setup.zip")).unwrap();

        // 現在のパスは更新される
        let found = store.find_by_stable_id(&id).unwrap().unwrap();
        assert_eq!(found.current_path, PathBuf::from("/apps/setup.zip"));

        // 旧パスは履歴として残る（消さない）
        let history: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM file_paths WHERE file_id = ?1", params![file_id], |r| r.get(0))
            .unwrap();
        assert_eq!(history, 2);
    }

    #[test]
    fn origins_stack_and_sort_by_confidence() {
        let store = Store::open_in_memory().unwrap();
        let file_id = store
            .insert_file(&sid("v", "k"), Path::new("/dl/a.zip"), 1, None, 0)
            .unwrap();

        let weak = Origin {
            url: Some("https://weak.example/a.zip".into()),
            referrer_url: None,
            host: Some("weak.example".into()),
            acquired_at: None,
            source: OriginSource::Gvfs,
            confidence: Confidence::Medium,
            browser: None,
            profile: None,
        };
        let strong = Origin {
            url: Some("https://strong.example/a.zip".into()),
            source: OriginSource::BrowserExt,
            confidence: Confidence::Certain,
            ..weak.clone()
        };

        store.add_origin(file_id, &weak).unwrap();
        store.add_origin(file_id, &strong).unwrap();

        let got = store.origins_of(file_id).unwrap();
        // 上書きせず両方積む。矛盾を握りつぶさないのが方針。
        assert_eq!(got.len(), 2);
        // 確度の高い方が先に来る。
        assert_eq!(got[0].source, OriginSource::BrowserExt);
        assert_eq!(got[1].source, OriginSource::Gvfs);
    }

    #[test]
    fn rejects_unknown_source() {
        // CHECK 制約が効いていること。未知の source を書けてしまうと、
        // 読み戻しで Corrupt になる行を作ってしまう。
        let store = Store::open_in_memory().unwrap();
        let file_id = store
            .insert_file(&sid("v", "k"), Path::new("/dl/a.zip"), 1, None, 0)
            .unwrap();
        let res = store.conn.execute(
            "INSERT INTO origins (file_id, source, confidence, recorded_at)
             VALUES (?1, 'telepathy', 'certain', 0)",
            params![file_id],
        );
        assert!(res.is_err());
    }
}
