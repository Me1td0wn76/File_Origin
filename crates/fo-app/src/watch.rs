//! 監視エンジン。イベントを受けて取り込みを走らせる。
//!
//! README §12 の当初案では `fo-watcher` という別クレートだったが、
//! やることは「イベントが来たら `ingest_file` を呼ぶ」であり、`fo-app` と同じ層。
//! 別クレートにしても依存グラフは変わらず、ビルド単位が増えるだけなので
//! ここに置く（Decision Log D12）。
//!
//! ## なぜ即座に取り込まないか
//!
//! ダウンロード 1 件でイベントは何十回も来る。ブラウザは `.crdownload` /
//! `.part` という一時名で書き、完了時に本来の名前へリネームする。
//! 書いている途中で取り込むと、未完成のファイルのハッシュとサイズを記録して
//! しまう。そこで **静穏時間（quiet period）** を置き、そのパスへのイベントが
//! 止まってから取り込む。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fo_platform::{FsEvent, FsWatcher, Platform};
use fo_store::Store;

use crate::ingest::{ingest_file, IngestOptions};
use crate::Result;

/// このパスへのイベントが止まってから取り込むまでの猶予。
///
/// 短すぎると書き込み途中を掴む。長すぎるとユーザーが「記録されない」と感じる。
/// ブラウザのリネーム完了を待つには 1 秒あれば足りる。
const QUIET_PERIOD: Duration = Duration::from_millis(1_000);

/// イベント待ちの 1 回あたりの上限。これが監視ループの応答性を決める。
const POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// ダウンロード中の一時ファイル。完了時にリネームされるので、これ自体は取り込まない。
///
/// 取り込んでしまうと「消えたファイル」の記録がゴミとして残る。
const IN_PROGRESS_SUFFIXES: &[&str] = &[
    ".crdownload", // Chrome / Edge
    ".part",       // Firefox
    ".partial",    // 旧 Edge / IE
    ".download",   // Safari
    ".tmp",
];

pub fn is_in_progress(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    IN_PROGRESS_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// 監視の結果。呼び出し側（デーモン）がログや統計に使う。
#[derive(Debug)]
pub enum WatchEvent<'a> {
    Ingested {
        path: &'a Path,
        verdict: &'a fo_core::Verdict,
        os_origins_recorded: usize,
        path_changed: bool,
    },
    /// 消えたので `missing` にした。
    MarkedMissing {
        path: &'a Path,
    },
    /// 取りこぼしたので再スキャンが要る。
    NeedsRescan,
    Failed {
        path: &'a Path,
        error: crate::Error,
    },
}

/// 保留中の取り込み。パスごとに「最後にイベントを見た時刻」を持つ。
#[derive(Default)]
pub struct Pending {
    seen: HashMap<PathBuf, Instant>,
    vanished: HashMap<PathBuf, Instant>,
}

impl Pending {
    fn touch(&mut self, path: PathBuf) {
        self.vanished.remove(&path);
        self.seen.insert(path, Instant::now());
    }

    fn mark_vanished(&mut self, path: PathBuf) {
        self.seen.remove(&path);
        self.vanished.insert(path, Instant::now());
    }

    /// 静穏時間を過ぎたものを取り出す。
    fn due(&mut self, now: Instant) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let split = |m: &mut HashMap<PathBuf, Instant>| {
            let ready: Vec<PathBuf> = m
                .iter()
                .filter(|(_, t)| now.duration_since(**t) >= QUIET_PERIOD)
                .map(|(p, _)| p.clone())
                .collect();
            for p in &ready {
                m.remove(p);
            }
            ready
        };
        (split(&mut self.seen), split(&mut self.vanished))
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty() && self.vanished.is_empty()
    }
}

/// 監視ループを 1 回分進める。
///
/// ループ本体をデーモンに持たせず、この関数を繰り返し呼ぶ形にしてある。
/// 停止要求の確認や他の仕事を挟めるようにするため。
pub fn tick(
    platform: &dyn Platform,
    store: &Store,
    watcher: &mut dyn FsWatcher,
    pending: &mut Pending,
    opts: IngestOptions,
    on_event: &mut dyn FnMut(WatchEvent<'_>),
) -> Result<()> {
    for event in watcher.poll(POLL_TIMEOUT)? {
        match event {
            FsEvent::Appeared(p) => {
                if !is_in_progress(&p) {
                    pending.touch(p);
                }
            }
            FsEvent::Vanished(p) => {
                if !is_in_progress(&p) {
                    pending.mark_vanished(p);
                }
            }
            FsEvent::Overflow => on_event(WatchEvent::NeedsRescan),
        }
    }

    let (appeared, vanished) = pending.due(Instant::now());

    for path in appeared {
        // 静穏時間の間に消えた（一時ファイルだった、すぐ移動された）場合は何もしない。
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        // **必ず正規化してから渡す。**
        // 監視ライブラリは登録した根のパスをそのまま前置して返すので、
        // 根が `C:/a/b` なら `C:/a/b\c.zip` のように区切りが混ざる。
        // `Path` の比較は区切りを同一視するが SQL の文字列比較はしないため、
        // 混ざったまま保存すると `fo show` が自分で入れた記録を引けなくなる。
        // スキャンと違い監視のイベントは低頻度なので、1 件ずつ払っても問題ない。
        let path = platform.paths().canonical(&path).unwrap_or(path);
        match ingest_file(platform, store, &path, &meta, opts) {
            Ok(ing) => on_event(WatchEvent::Ingested {
                path: &path,
                verdict: &ing.verdict,
                os_origins_recorded: ing.os_origins_recorded,
                path_changed: ing.path_changed,
            }),
            Err(error) => on_event(WatchEvent::Failed { path: &path, error }),
        }
    }

    for path in vanished {
        // 移動の前半だった場合、静穏時間の間に新しい場所で Appeared が来て
        // 同一識別子で解決されている。ここで実在を再確認して、
        // 本当に消えたものだけ missing にする。
        if path.exists() {
            continue;
        }
        // 消えたファイルは canonical() を通せない（実体が要る）ので、
        // 区切りだけ揃えて DB の文字列と突き合わせる。
        let path = normalize_separators(&path);
        match store.mark_missing_by_path(&path) {
            Ok(true) => on_event(WatchEvent::MarkedMissing { path: &path }),
            Ok(false) => {}
            Err(e) => on_event(WatchEvent::Failed {
                path: &path,
                error: e.into(),
            }),
        }
    }

    Ok(())
}

/// 実体が無いパスの区切りを、その OS の正規の形に揃える。
///
/// `canonical()` は実在するファイルにしか使えないので、消えたファイルの
/// 突き合わせにはこちらを使う。内容は変えず、区切り文字だけ直す。
pub fn normalize_separators(path: &Path) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '/' {
        return path.to_path_buf();
    }
    match path.to_str() {
        Some(s) if s.contains('/') => PathBuf::from(s.replace('/', "\\")),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mixed_separators() {
        // 監視ライブラリが返す混在パスを、DB の文字列と突き合わせられる形にする。
        // これが揃っていないと、自分で入れた記録を `fo show` が引けない。
        let mixed = Path::new(r"C:/dl/sub\a.zip");
        let fixed = normalize_separators(mixed);
        // OS そのものを判定せず、std が持つ区切り文字の値を見る。
        // 条件コンパイルで分岐すると arch-guard に止められる — そしてそれは正しい。
        // 知りたいのは「区切り文字は何か」であって「どの OS か」ではない。
        if std::path::MAIN_SEPARATOR == '\\' {
            assert_eq!(fixed, PathBuf::from(r"C:\dl\sub\a.zip"));
        } else {
            assert_eq!(fixed, mixed);
        }
    }

    #[test]
    fn detects_in_progress_downloads() {
        assert!(is_in_progress(Path::new("/dl/a.zip.crdownload")));
        assert!(is_in_progress(Path::new("/dl/a.zip.part")));
        assert!(is_in_progress(Path::new(r"C:\dl\A.ZIP.PARTIAL")));
        assert!(!is_in_progress(Path::new("/dl/a.zip")));
        assert!(!is_in_progress(Path::new("/dl/partial-report.pdf")));
    }

    #[test]
    fn quiet_period_defers_then_releases() {
        let mut p = Pending::default();
        p.touch(PathBuf::from("/dl/a.zip"));

        // 直後はまだ取り込まない
        let (ready, _) = p.due(Instant::now());
        assert!(ready.is_empty());
        assert!(!p.is_empty());

        // 静穏時間を過ぎたことにする
        let (ready, _) = p.due(Instant::now() + QUIET_PERIOD);
        assert_eq!(ready, vec![PathBuf::from("/dl/a.zip")]);
        assert!(p.is_empty());
    }

    #[test]
    fn reappearing_cancels_vanish() {
        // 移動: Vanished(旧) → Appeared(新) ではなく、同じパスが消えて戻る場合。
        let mut p = Pending::default();
        p.mark_vanished(PathBuf::from("/dl/a.zip"));
        p.touch(PathBuf::from("/dl/a.zip"));

        let (ready, gone) = p.due(Instant::now() + QUIET_PERIOD);
        assert_eq!(ready, vec![PathBuf::from("/dl/a.zip")]);
        assert!(gone.is_empty(), "消えた扱いは取り消される");
    }
}
