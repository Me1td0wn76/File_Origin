//! ディレクトリを走査して取り込む。
//!
//! 進捗はイベントとして呼び出し側に渡す。CLI は行を出し、GUI はプログレスバーを動かす。
//! 集計や表示をここでやらないのは、UI ごとに欲しい形が違うため。

use std::path::Path;

use fo_core::Verdict;
use fo_platform::Platform;
use fo_store::Store;

use crate::ingest::{ingest_file, IngestOptions};
use crate::{Error, Result};

/// 走査の深さ上限。シンボリックリンクの輪や、意図せず `/` を走査してしまう
/// 事故で止まらなくなるのを防ぐ。
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub recursive: bool,
    pub ingest: IngestOptions,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            recursive: true,
            ingest: IngestOptions::default(),
        }
    }
}

/// 走査中に起きたこと。
#[derive(Debug)]
pub enum ScanEvent<'a> {
    /// 1 ファイルを処理した。
    File {
        path: &'a Path,
        verdict: &'a Verdict,
        os_origins_recorded: usize,
    },
    /// 読めなかった。処理は継続する。
    Error { path: &'a Path, error: Error },
}

/// 何ファイルごとにコミットするか。
///
/// 1 ファイルずつ autocommit すると fsync の回数分だけ遅い（6,900 ファイルで 31 秒）。
/// 全部を 1 トランザクションにすると、途中で落ちたとき何も残らない。
/// その間を取る。この値で途中終了しても、直前のコミットまでは記録が残る。
const COMMIT_EVERY: usize = 500;

/// ディレクトリを走査する。`root` は未正規化でよい（ここで正規化する）。
/// 正規化後のルートを返す。
pub fn scan_dir(
    platform: &dyn Platform,
    store: &Store,
    root: &Path,
    opts: ScanOptions,
    on_event: &mut dyn FnMut(ScanEvent<'_>),
) -> Result<std::path::PathBuf> {
    let root = platform
        .paths()
        .canonical(root)
        .map_err(|source| Error::Canonicalize {
            path: root.to_path_buf(),
            source,
        })?;

    let mut batch = Batcher::begin(store)?;
    walk(platform, store, &root, opts, 0, on_event, &mut batch);
    batch.finish()?;
    Ok(root)
}

/// 走査中の書き込みをまとめる。`COMMIT_EVERY` 件ごとに区切ってコミットする。
struct Batcher<'a> {
    store: &'a Store,
    current: Option<fo_store::Batch<'a>>,
    pending: usize,
}

impl<'a> Batcher<'a> {
    fn begin(store: &'a Store) -> Result<Self> {
        Ok(Self {
            store,
            current: Some(store.batch()?),
            pending: 0,
        })
    }

    /// 1 ファイル処理したことを記録し、必要なら区切る。
    fn tick(&mut self) -> Result<()> {
        self.pending += 1;
        if self.pending >= COMMIT_EVERY {
            self.finish()?;
            self.current = Some(self.store.batch()?);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(b) = self.current.take() {
            b.commit()?;
        }
        self.pending = 0;
        Ok(())
    }
}

fn walk(
    platform: &dyn Platform,
    store: &Store,
    dir: &Path,
    opts: ScanOptions,
    depth: usize,
    on_event: &mut dyn FnMut(ScanEvent<'_>),
    batch: &mut Batcher<'_>,
) {
    if depth > MAX_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            // 読めないディレクトリで走査全体を止めない。
            // ダウンロードフォルダの一部が読めないのは普通にある。
            on_event(ScanEvent::Error {
                path: dir,
                error: e.into(),
            });
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // シンボリックリンクは辿らない。リンク先が走査範囲の外にあったり、
        // 輪を作っていたりするため。実体は本来の場所で拾う。
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                on_event(ScanEvent::Error {
                    path: &path,
                    error: e.into(),
                });
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            continue;
        }

        if meta.is_dir() {
            if opts.recursive {
                walk(platform, store, &path, opts, depth + 1, on_event, batch);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }

        match ingest_file(platform, store, &path, &meta, opts.ingest) {
            Ok(ingested) => on_event(ScanEvent::File {
                path: &path,
                verdict: &ingested.verdict,
                os_origins_recorded: ingested.os_origins_recorded,
            }),
            Err(error) => on_event(ScanEvent::Error { path: &path, error }),
        }
        // コミットに失敗したら DB 側の問題なので、走査を続けても意味がない。
        // ただし walk は結果を返さない設計なので、イベントで知らせて打ち切る。
        if let Err(error) = batch.tick() {
            on_event(ScanEvent::Error { path: &path, error });
            return;
        }
    }
}
