-- ファイル名検索用にベース名を別列で持つ。
--
-- フルパスに LIKE を当てると、`setup` がディレクトリ名 `setup\foo.zip` にも
-- 当たってしまう。ベース名を切り出しておけば LIKE '%setup%' が意図どおり効く。
-- 既存行の埋め戻しはパス区切りが OS 依存なので SQL ではなく Rust 側で行う
-- （Store::backfill_path_names）。

ALTER TABLE file_paths ADD COLUMN name TEXT;

CREATE INDEX IF NOT EXISTS idx_paths_name ON file_paths(name);
