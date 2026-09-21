//! SQLite 永続化層。
//!
//! **書き込み口はデーモン 1 つに集約する。** Native Messaging ホストは
//! ブラウザが複数プロファイルから同時に spawn するため、そこから直接 DB を
//! 触ると書き込みが競合する（README §9.1）。
//!
//! DB は暗号化しない（ADR-0006）。OS のファイル権限に依存する。

use std::path::{Path, PathBuf};

use fo_core::model::{
    Confidence, Digest, FileRecord, FileStatus, Origin, OriginSource, PathEntry, SearchHit,
    SearchQuery,
};
use fo_platform::{FileKey, StableFileId, VolumeId};
use rusqlite::{params, Connection, OptionalExtension};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("データベース: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("データディレクトリを作成できない {path}: {source}")]
    DataDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("DB に想定外の値が入っている: {0}")]
    Corrupt(String),
}

/// マイグレーション。追加するときは末尾に足し、既存を書き換えない。
/// 既に適用済みの DB を壊さないため。
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    (
        "0002_path_name",
        include_str!("../migrations/0002_path_name.sql"),
    ),
];

pub struct Store {
    conn: Connection,
}

/// 進行中のトランザクション。`commit()` しないままドロップすると巻き戻る。
pub struct Batch<'a>(rusqlite::Transaction<'a>);

impl Batch<'_> {
    pub fn commit(self) -> Result<()> {
        Ok(self.0.commit()?)
    }
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
        // SQL だけでは埋められない列を Rust 側で補う。冪等なので毎回呼んでよい。
        self.backfill_path_names()?;
        Ok(())
    }

    /// `file_paths.name` が NULL の行にベース名を入れる（0002 の埋め戻し）。
    /// パス区切りが OS 依存なので SQL ではなく `Path::file_name` で切る。
    fn backfill_path_names(&self) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM file_paths WHERE name IS NULL")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);

        for (id, path) in rows {
            let name = basename(Path::new(&path));
            self.conn.execute(
                "UPDATE file_paths SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
        }
        Ok(())
    }

    /// 複数の書き込みを 1 つのトランザクションにまとめる。
    ///
    /// SQLite は autocommit だと 1 文ごとに fsync するため、数千ファイルの初回走査が
    /// 数十秒かかる。まとめれば秒単位になる。`commit()` を呼ばずに落とすと巻き戻る。
    pub fn batch(&self) -> Result<Batch<'_>> {
        Ok(Batch(self.conn.unchecked_transaction()?))
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
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// 新しいファイルを記録し、現在のパスを 1 件登録する。
    ///
    /// `derived_from` はコピー元。コピーを別ファイルとして扱いつつ、
    /// 入手元の系譜を辿れるようにする（README §8.1 の 3 段目）。
    pub fn insert_file(
        &self,
        id: &StableFileId,
        path: &Path,
        size: u64,
        sha256: Option<&Digest>,
        mtime: i64,
        derived_from: Option<i64>,
    ) -> Result<i64> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO files
                 (volume_id, file_key, size, sha256, mtime, status, derived_from,
                  first_seen_at, last_verified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'present', ?6, ?7, ?7)",
            params![
                id.volume.0,
                id.file.0,
                size as i64,
                sha256.map(|d| d.as_str()),
                mtime,
                derived_from,
                ts
            ],
        )?;
        let file_id = self.conn.last_insert_rowid();
        self.record_path(file_id, path)?;
        Ok(file_id)
    }

    /// 安定識別子を差し替える。別ボリュームへの移動で識別子が変わったときに使う。
    /// ハッシュ一致で同一と判定した結果を反映する（README §8.1 の 2 段目）。
    pub fn update_stable_id(&self, file_id: i64, id: &StableFileId) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET volume_id = ?1, file_key = ?2, last_verified_at = ?3
             WHERE id = ?4",
            params![id.volume.0, id.file.0, now(), file_id],
        )?;
        Ok(())
    }

    /// 内容が更新されたことを記録する。
    /// 古いダイジェストは `file_versions` に版として残し、`files` を新しい値にする。
    pub fn update_content(
        &self,
        file_id: i64,
        sha256: &Digest,
        size: u64,
        mtime: i64,
    ) -> Result<()> {
        let ts = now();
        // 旧版を履歴へ。sha256 が NULL（未計算）だった場合は版として残すものが無い。
        self.conn.execute(
            "INSERT INTO file_versions (file_id, sha256, size, observed_at)
             SELECT id, sha256, size, ?2 FROM files
             WHERE id = ?1 AND sha256 IS NOT NULL",
            params![file_id, ts],
        )?;
        self.conn.execute(
            "UPDATE files SET sha256 = ?1, size = ?2, mtime = ?3, last_verified_at = ?4
             WHERE id = ?5",
            params![sha256.as_str(), size as i64, mtime, ts, file_id],
        )?;
        Ok(())
    }

    /// ハッシュを後から埋める（遅延計算・ADR-0007）。
    pub fn set_sha256(&self, file_id: i64, sha256: &Digest) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET sha256 = ?1, last_verified_at = ?2 WHERE id = ?3",
            params![sha256.as_str(), now(), file_id],
        )?;
        Ok(())
    }

    /// 現在のパスを記録する。以前のパスは履歴として残す（消さない）。
    pub fn record_path(&self, file_id: i64, path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE file_paths SET is_current = 0 WHERE file_id = ?1 AND is_current = 1",
            params![file_id],
        )?;
        self.conn.execute(
            "INSERT INTO file_paths (file_id, path, name, is_current, observed_at)
             VALUES (?1, ?2, ?3, 1, ?4)",
            params![file_id, path.to_string_lossy(), basename(path), now()],
        )?;
        Ok(())
    }

    /// 条件に合うファイルを、最初に見た日時の新しい順に返す。
    ///
    /// SQL は固定で、指定の無い条件は `:x IS NULL` で素通しにする。
    /// 動的に組み立てないのは、条件の組み合わせごとにテストしなくて済むようにするため。
    ///
    /// ファイル名は **過去の名前も含めて** 当てる。「昔 setup.zip だったあれはどこ？」に
    /// 答えるのがこのツールの約束で、リネーム後の名前しか引けないなら履歴を持つ意味がない。
    /// 表示するパスは現在のもの。
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let name_like = q.name.as_deref().map(glob_to_like);
        let url_like = q.url.as_deref().map(|u| format!("%{}%", escape_like(u)));
        let host = q.host.as_deref().map(str::to_ascii_lowercase);
        let host_sub = host.as_deref().map(|h| format!("%.{}", escape_like(h)));
        let limit = if q.limit == 0 { -1 } else { q.limit as i64 };

        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT
                    f.id, f.volume_id, f.file_key, f.size, f.sha256, f.mtime,
                    f.status, f.derived_from, p.path, f.first_seen_at
             FROM files f
             JOIN file_paths p ON p.file_id = f.id AND p.is_current = 1
             LEFT JOIN origins o ON o.file_id = f.id
             WHERE (:name IS NULL OR EXISTS (
                        SELECT 1 FROM file_paths ph
                        WHERE ph.file_id = f.id AND ph.name LIKE :name ESCAPE '!'))
               AND (:url  IS NULL OR o.url LIKE :url ESCAPE '!'
                                  OR o.referrer_url LIKE :url ESCAPE '!')
               AND (:host IS NULL OR o.host = :host OR o.host LIKE :host_sub ESCAPE '!')
               AND (:since IS NULL OR COALESCE(o.acquired_at, f.first_seen_at) >= :since)
               AND (:until IS NULL OR COALESCE(o.acquired_at, f.first_seen_at) <  :until)
               AND (:sha  IS NULL OR f.sha256 = :sha)
             ORDER BY f.first_seen_at DESC, f.id DESC
             LIMIT :limit",
        )?;

        let rows = stmt.query_map(
            rusqlite::named_params! {
                ":name": name_like,
                ":url": url_like,
                ":host": host,
                ":host_sub": host_sub,
                ":since": q.since,
                ":until": q.until,
                ":sha": q.sha256.as_ref().map(|d| d.as_str()),
                ":limit": limit,
            },
            row_to_file,
        )?;
        let records: Vec<FileRecord> = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect::<Result<_>>()?;

        let mut hits = Vec::with_capacity(records.len());
        for record in records {
            let best_origin = self.origins_of(record.id)?.into_iter().next();
            hits.push(SearchHit {
                record,
                best_origin,
            });
        }
        Ok(hits)
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

    /// id でファイルを引く。`derived_from` の系譜を辿るときに使う。
    pub fn get_file(&self, file_id: i64) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT f.id, f.volume_id, f.file_key, f.size, f.sha256, f.mtime,
                        f.status, f.derived_from,
                        COALESCE((SELECT p.path FROM file_paths p
                                  WHERE p.file_id = f.id AND p.is_current = 1
                                  LIMIT 1), '')
                 FROM files f
                 WHERE f.id = ?1",
                params![file_id],
                row_to_file,
            )
            .optional()?
            .transpose()
    }

    /// パス履歴を新しい順に返す。先頭が現在のパス。
    pub fn path_history(&self, file_id: i64) -> Result<Vec<PathEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, is_current, observed_at
             FROM file_paths WHERE file_id = ?1
             ORDER BY is_current DESC, observed_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![file_id], |row| {
            Ok(PathEntry {
                path: PathBuf::from(row.get::<_, String>(0)?),
                is_current: row.get::<_, i64>(1)? != 0,
                observed_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

/// パスからベース名を取る。取れなければパス全体（ルートなど）。
fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// SQL `LIKE ... ESCAPE` のエスケープ文字。
///
/// バックスラッシュにしないのは、Windows のパスやファイル名に普通に含まれるため。
/// `!` はファイル名にも URL にもほぼ現れず、現れても正しくエスケープされる。
const LIKE_ESCAPE: char = '!';

/// glob（`*` `?`）を SQL LIKE（`%` `_`）に変換する。
/// LIKE のメタ文字 `%` `_` と、エスケープ文字自身はエスケープする。
fn glob_to_like(glob: &str) -> String {
    let mut out = String::with_capacity(glob.len() + 4);
    for c in glob.chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' | '_' | LIKE_ESCAPE => {
                out.push(LIKE_ESCAPE);
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// LIKE の中に部分文字列として埋め込むための最小限のエスケープ。
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(c, '%' | '_' | LIKE_ESCAPE) {
            out.push(LIKE_ESCAPE);
        }
        out.push(c);
    }
    out
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
    fn glob_translates_to_like() {
        assert_eq!(glob_to_like("setup*"), "setup%");
        assert_eq!(glob_to_like("a?c"), "a_c");
        // LIKE のメタ文字はエスケープされる
        assert_eq!(glob_to_like("100%_off"), "100!%!_off");
        // バックスラッシュは Windows のパスに出るので素通し
        assert_eq!(glob_to_like(r"a\b"), r"a\b");
    }

    #[test]
    fn backfill_fills_missing_names() {
        let store = Store::open_in_memory().unwrap();
        let file_id = store
            .insert_file(&sid("v", "k"), Path::new("/dl/setup.zip"), 1, None, 0, None)
            .unwrap();
        // 0002 適用前の DB を再現する
        store
            .conn
            .execute("UPDATE file_paths SET name = NULL", [])
            .unwrap();
        store.backfill_path_names().unwrap();

        let name: String = store
            .conn
            .query_row(
                "SELECT name FROM file_paths WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "setup.zip");
    }

    /// 検索テスト用の DB。3 ファイル・入手元つき。
    fn seeded() -> Store {
        let store = Store::open_in_memory().unwrap();
        let mk =
            |v: &str, path: &str, url: Option<&str>, host: Option<&str>, acquired: Option<i64>| {
                let id = store
                    .insert_file(&sid(v, v), Path::new(path), 1, None, 0, None)
                    .unwrap();
                if let Some(u) = url {
                    store
                        .add_origin(
                            id,
                            &Origin {
                                url: Some(u.into()),
                                referrer_url: Some("https://ref.example/page".into()),
                                host: host.map(str::to_string),
                                acquired_at: acquired,
                                source: OriginSource::ZoneIdentifier,
                                confidence: Confidence::High,
                                browser: None,
                                profile: None,
                            },
                        )
                        .unwrap();
                }
                id
            };
        mk(
            "a",
            "/dl/setup.zip",
            Some("https://cdn.example.com/setup.zip"),
            Some("cdn.example.com"),
            Some(1_000),
        );
        mk(
            "b",
            "/dl/tool.exe",
            Some("https://other.net/tool.exe"),
            Some("other.net"),
            Some(2_000),
        );
        mk("c", "/dl/setup-notes.txt", None, None, None);
        store
    }

    #[test]
    fn search_by_name_glob() {
        let store = seeded();
        let q = SearchQuery {
            name: Some("setup*".into()),
            ..Default::default()
        };
        let hits = store.search(&q).unwrap();
        let mut names: Vec<String> = hits
            .iter()
            .map(|h| {
                h.record
                    .current_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        // 入手元の無いファイルも名前で引ける（LEFT JOIN）
        assert_eq!(names, vec!["setup-notes.txt", "setup.zip"]);
    }

    #[test]
    fn search_by_name_matches_past_names() {
        let store = seeded();
        // setup.zip をリネームする。現在の名前は installer.zip になる。
        let id = store
            .find_by_path(Path::new("/dl/setup.zip"))
            .unwrap()
            .unwrap()
            .id;
        store
            .record_path(id, Path::new("/apps/installer.zip"))
            .unwrap();

        // 昔の名前で引けて、表示は現在のパス。
        let hits = store
            .search(&SearchQuery {
                name: Some("setup.zip".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].record.current_path,
            PathBuf::from("/apps/installer.zip")
        );

        // 新しい名前でも当然引ける。
        let hits = store
            .search(&SearchQuery {
                name: Some("installer*".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_by_url_substring_and_host_subdomain() {
        let store = seeded();
        let by_url = store
            .search(&SearchQuery {
                url: Some("other.net".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_url.len(), 1);
        assert!(by_url[0].record.current_path.ends_with("tool.exe"));

        // cdn.example.com は example.com のサブドメインとして当たる
        let by_host = store
            .search(&SearchQuery {
                host: Some("example.com".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_host.len(), 1);
        assert!(by_host[0].record.current_path.ends_with("setup.zip"));
        assert_eq!(
            by_host[0]
                .best_origin
                .as_ref()
                .and_then(|o| o.url.clone())
                .as_deref(),
            Some("https://cdn.example.com/setup.zip")
        );
    }

    #[test]
    fn search_by_acquired_range_and_combined() {
        let store = seeded();
        // 1500 以降 → tool.exe（2000）だけ。setup-notes は acquired が無いので
        // first_seen_at（now）で代用され、範囲外になる…わけではなく now は 1500 より大きい。
        // よって since=1500 では tool.exe と setup-notes の 2 件。
        let since = store
            .search(&SearchQuery {
                since: Some(1_500),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(since.len(), 2);

        // until=1500 → setup.zip（1000）だけ
        let until = store
            .search(&SearchQuery {
                until: Some(1_500),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(until.len(), 1);
        assert!(until[0].record.current_path.ends_with("setup.zip"));

        // 条件は AND: name=setup* かつ host=example.com → setup.zip だけ
        let both = store
            .search(&SearchQuery {
                name: Some("setup*".into()),
                host: Some("example.com".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(both.len(), 1);
    }

    #[test]
    fn search_limit_and_no_filter() {
        let store = seeded();
        assert_eq!(store.search(&SearchQuery::default()).unwrap().len(), 3);
        let limited = store
            .search(&SearchQuery {
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn records_and_reads_back_a_file() {
        let store = Store::open_in_memory().unwrap();
        let id = sid("vol1", "key1");
        let file_id = store
            .insert_file(
                &id,
                Path::new("/dl/setup.zip"),
                1234,
                Some(&Digest("aa".into())),
                99,
                None,
            )
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
            .insert_file(&id, Path::new("/dl/setup.zip"), 10, None, 0, None)
            .unwrap();

        store
            .record_path(file_id, Path::new("/apps/setup.zip"))
            .unwrap();

        // 現在のパスは更新される
        let found = store.find_by_stable_id(&id).unwrap().unwrap();
        assert_eq!(found.current_path, PathBuf::from("/apps/setup.zip"));

        // 旧パスは履歴として残る（消さない）
        let history: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM file_paths WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(history, 2);
    }

    #[test]
    fn origins_stack_and_sort_by_confidence() {
        let store = Store::open_in_memory().unwrap();
        let file_id = store
            .insert_file(&sid("v", "k"), Path::new("/dl/a.zip"), 1, None, 0, None)
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
            .insert_file(&sid("v", "k"), Path::new("/dl/a.zip"), 1, None, 0, None)
            .unwrap();
        let res = store.conn.execute(
            "INSERT INTO origins (file_id, source, confidence, recorded_at)
             VALUES (?1, 'telepathy', 'certain', 0)",
            params![file_id],
        );
        assert!(res.is_err());
    }
}
