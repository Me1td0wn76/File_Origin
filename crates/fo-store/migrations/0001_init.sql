-- File Origin 初期スキーマ
--
-- 設計は README §10 を参照。要点:
--   * files と file_paths を分けるのは、パスが属性ではなく履歴だから。
--     「以前どこにあったか」に答えられるようにする。
--   * origins は 1 ファイルに複数行。再ダウンロードしても上書きせず積む。
--     矛盾する情報も握りつぶさず、source と confidence を添えて両方残す。
--   * sha256 は NULL 許容。記録とハッシュ計算を切り離すため（ADR-0007）。

CREATE TABLE IF NOT EXISTS files (
    id               INTEGER PRIMARY KEY,
    volume_id        TEXT    NOT NULL,   -- Win: VolumeSerialNumber / Linux: st_dev
    file_key         TEXT    NOT NULL,   -- Win: FileId128          / Linux: st_ino
    size             INTEGER NOT NULL,
    sha256           TEXT,               -- 計算前は NULL（遅延計算）
    mtime            INTEGER NOT NULL,
    status           TEXT    NOT NULL CHECK (status IN ('present', 'missing', 'deleted')),
    derived_from     INTEGER REFERENCES files(id),   -- コピー元
    first_seen_at    INTEGER NOT NULL,
    last_verified_at INTEGER NOT NULL,
    UNIQUE (volume_id, file_key)
);

CREATE INDEX IF NOT EXISTS idx_files_sha256 ON files(sha256);
CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);

CREATE TABLE IF NOT EXISTS file_paths (
    id          INTEGER PRIMARY KEY,
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    is_current  INTEGER NOT NULL CHECK (is_current IN (0, 1)),
    observed_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_paths_path    ON file_paths(path);
CREATE INDEX IF NOT EXISTS idx_paths_current ON file_paths(file_id, is_current);

CREATE TABLE IF NOT EXISTS origins (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    url          TEXT,
    referrer_url TEXT,
    host         TEXT,                   -- URL から導出。ホスト単位の検索用
    acquired_at  INTEGER,
    source       TEXT    NOT NULL CHECK (source IN (
                     'browser_ext', 'zone_identifier', 'xattr',
                     'gvfs', 'manual', 'history_db')),
    confidence   TEXT    NOT NULL CHECK (confidence IN (
                     'certain', 'high', 'medium', 'low')),
    browser      TEXT,
    profile      TEXT,
    recorded_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_origins_file ON origins(file_id);
CREATE INDEX IF NOT EXISTS idx_origins_host ON origins(host);
CREATE INDEX IF NOT EXISTS idx_origins_url  ON origins(url);

-- 内容の版履歴。同一 file_id で SHA-256 が変わった場合に追記する。
CREATE TABLE IF NOT EXISTS file_versions (
    id          INTEGER PRIMARY KEY,
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    sha256      TEXT    NOT NULL,
    size        INTEGER NOT NULL,
    observed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
    file_id    INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    body       TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    PRIMARY KEY (file_id, tag)
);

CREATE TABLE IF NOT EXISTS scan_roots (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    recursive    INTEGER NOT NULL CHECK (recursive IN (0, 1)),
    last_scan_at INTEGER
);

-- 変更ジャーナルの再開位置。Windows の USN カーソルを保持する。
-- Linux では未使用（fanotify に再開位置の概念が無いため空のまま）。
CREATE TABLE IF NOT EXISTS journal_state (
    volume_id  TEXT PRIMARY KEY,
    cursor     TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);
