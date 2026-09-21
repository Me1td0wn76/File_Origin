//! ファイル監視。
//!
//! 実装は `notify` クレートに任せる。OS 差（Windows: `ReadDirectoryChangesW` /
//! Linux: `inotify`）はその中に閉じているが、**呼び出し側に notify の型を見せない**。
//! 見せると `fo-app` が監視バックエンドに縛られ、将来 USN Journal や fanotify に
//! 差し替えるときにアプリ層まで波及する（設計方針 P1）。
//!
//! ## リネームをイベント対にしない理由
//!
//! `notify` はリネームを「From」「To」の 2 イベントで返し、対応付けには
//! トラッキング ID の突き合わせが要る。File Origin はこれをやらない。
//!
//! 安定識別子（`StableFileId`）は移動しても変わらないので、`To` 側を
//! 通常の «現れた» イベントとして取り込めば、同一性判定のはしご 1 段目が
//! 「同じファイルが別の場所にある」と判定してパスを更新する。
//! 対応付けを自前でやるより、既にテスト済みの判定に委ねる方が確実。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::{Error, Result};

/// 監視で観測した出来事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsEvent {
    /// 現れた、または内容が変わった。
    /// 作成と変更を区別しないのは、取り込み側がどちらでも同じ処理をするため
    /// （安定識別子で引き直して、必要ならパスと内容を更新する）。
    Appeared(PathBuf),
    /// 消えた。移動の前半のこともあるので、即座に削除と決めつけない。
    Vanished(PathBuf),
    /// イベントを取りこぼした。差分スキャンでの補正が要る。
    Overflow,
}

impl FsEvent {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Appeared(p) | Self::Vanished(p) => Some(p),
            Self::Overflow => None,
        }
    }
}

pub trait FsWatcher: Send {
    /// 監視対象を追加する。
    fn watch(&mut self, root: &Path, recursive: bool) -> Result<()>;
    /// 監視対象を外す。
    fn unwatch(&mut self, root: &Path) -> Result<()>;
    /// 次のイベント群を取る。`timeout` まで待ち、無ければ空を返す。
    ///
    /// まとめて返すのは、ダウンロード 1 つで何十回も書き込みイベントが出るため。
    /// 呼び出し側が重複を畳める。
    fn poll(&mut self, timeout: Duration) -> Result<Vec<FsEvent>>;
}

/// `notify` を使った実装。
pub struct NotifyWatcher {
    inner: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
}

impl NotifyWatcher {
    pub fn new() -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let inner = notify::recommended_watcher(move |res| {
            // 受け手が落ちていたら捨てる。監視スレッドを道連れにしない。
            let _ = tx.send(res);
        })
        .map_err(to_error)?;
        Ok(Self { inner, rx })
    }
}

impl FsWatcher for NotifyWatcher {
    fn watch(&mut self, root: &Path, recursive: bool) -> Result<()> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        self.inner.watch(root, mode).map_err(to_error)
    }

    fn unwatch(&mut self, root: &Path) -> Result<()> {
        self.inner.unwatch(root).map_err(to_error)
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<FsEvent>> {
        let mut out = Vec::new();

        // 最初の 1 件は timeout まで待つ。以降は溜まっている分だけ浚う。
        match self.rx.recv_timeout(timeout) {
            Ok(res) => push(&mut out, res),
            Err(RecvTimeoutError::Timeout) => return Ok(out),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::Unsupported("監視スレッドが停止しました"))
            }
        }
        while let Ok(res) = self.rx.try_recv() {
            push(&mut out, res);
        }
        Ok(out)
    }
}

fn push(out: &mut Vec<FsEvent>, res: notify::Result<notify::Event>) {
    use notify::EventKind;

    let event = match res {
        Ok(e) => e,
        // バッファ溢れなどはイベントとして返す。黙って捨てると
        // 「監視しているのに記録されない」状態になる。
        Err(_) => {
            out.push(FsEvent::Overflow);
            return;
        }
    };

    for path in event.paths {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => out.push(FsEvent::Appeared(path)),
            EventKind::Remove(_) => out.push(FsEvent::Vanished(path)),
            // Access は読み取りなので記録に影響しない。Any/Other は判断できないので
            // 「現れたかもしれない」側に倒す — 取りこぼすより余分に確認する方が安全。
            EventKind::Access(_) => {}
            EventKind::Any | EventKind::Other => out.push(FsEvent::Appeared(path)),
        }
    }
}

fn to_error(e: notify::Error) -> Error {
    match e.kind {
        notify::ErrorKind::PathNotFound => Error::NotFound(
            e.paths
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("?")),
        ),
        notify::ErrorKind::MaxFilesWatch => Error::Unsupported(
            "監視できるファイル数の上限に達しました（Linux: fs.inotify.max_user_watches）",
        ),
        _ => Error::Io(std::io::Error::other(e.to_string())),
    }
}
