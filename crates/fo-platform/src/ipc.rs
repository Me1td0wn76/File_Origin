//! ローカル IPC。
//!
//! CLI・GUI・Native Messaging ホストがデーモンに話しかける経路。
//! `interprocess` が Windows の名前付きパイプと Unix ドメインソケットに写す。
//!
//! ## 名前の選び方が OS で違う（そしてそれは安全性の問題）
//!
//! - **Windows**: 名前空間（`\\.\pipe\file-origin`）。ファイルシステム上に実体が無い。
//! - **Linux**: **ファイルパス**（`$XDG_RUNTIME_DIR/file-origin.sock`）。
//!   Linux にも抽象名前空間ソケット（`@name`）はあるが、**ファイル権限を持たない** —
//!   同じネットワーク名前空間のどのプロセスからでも接続できてしまう。
//!   `$XDG_RUNTIME_DIR` は 0700 なので、そこに置く方が守れる（README §11）。
//!
//! この差があるため、名前の組み立ては OS 実装側（`windows/ipc.rs` / `linux/ipc.rs`）が持つ。

use std::io::{Read, Write};
use std::path::PathBuf;

use interprocess::local_socket::{
    prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream,
};

use crate::{Error, Result};

/// エンドポイントの名前。OS 実装がどちらかを選ぶ。
#[derive(Debug, Clone)]
pub enum IpcName {
    /// OS の名前空間（Windows の名前付きパイプ）。
    Namespaced(String),
    /// ファイルシステム上のパス（Unix ドメインソケット）。
    FilePath(PathBuf),
}

impl IpcName {
    fn to_name(&self) -> Result<Name<'_>> {
        match self {
            Self::Namespaced(s) => s.as_str().to_ns_name::<GenericNamespaced>(),
            Self::FilePath(p) => p.as_path().to_fs_name::<GenericFilePath>(),
        }
        .map_err(Error::Io)
    }

    pub fn display(&self) -> String {
        match self {
            Self::Namespaced(s) => format!(r"\\.\pipe\{s}"),
            Self::FilePath(p) => p.display().to_string(),
        }
    }
}

pub trait IpcTransport: Send + Sync {
    /// サーバとして待ち受ける。既に動いているデーモンがあれば失敗する。
    fn bind(&self) -> Result<Box<dyn IpcListener>>;
    /// クライアントとして接続する。デーモンが居なければ失敗する。
    fn connect(&self) -> Result<Box<dyn IpcStream>>;
    /// `fo doctor` に出す表示名。
    fn endpoint_display(&self) -> String;
}

pub trait IpcListener: Send {
    /// 次の接続を待つ。
    fn accept(&self) -> Result<Box<dyn IpcStream>>;
}

/// 接続 1 本。行区切り JSON を読み書きする（プロトコルは `fo-ipc`）。
pub trait IpcStream: Read + Write + Send {}

pub struct LocalSocket {
    name: IpcName,
}

impl LocalSocket {
    pub fn new(name: IpcName) -> Self {
        Self { name }
    }
}

impl IpcTransport for LocalSocket {
    fn bind(&self) -> Result<Box<dyn IpcListener>> {
        // 置き場所が無ければ作る。XDG_RUNTIME_DIR が無い環境で
        // 一時ディレクトリ配下を使う場合に要る。
        if let IpcName::FilePath(p) = &self.name {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // 前回の異常終了で残ったソケットファイルは、接続できないなら捨てる。
            // 生きているデーモンがあれば connect が成功するので、そのときは消さない。
            if p.exists() && self.connect().is_err() {
                let _ = std::fs::remove_file(p);
            }
        }
        let listener = ListenerOptions::new()
            .name(self.name.to_name()?)
            .create_sync()
            .map_err(Error::Io)?;
        Ok(Box::new(Listener(listener)))
    }

    fn connect(&self) -> Result<Box<dyn IpcStream>> {
        let stream = Stream::connect(self.name.to_name()?).map_err(Error::Io)?;
        Ok(Box::new(Conn(stream)))
    }

    fn endpoint_display(&self) -> String {
        self.name.display()
    }
}

struct Listener(interprocess::local_socket::Listener);

impl IpcListener for Listener {
    fn accept(&self) -> Result<Box<dyn IpcStream>> {
        let stream = self.0.accept().map_err(Error::Io)?;
        Ok(Box::new(Conn(stream)))
    }
}

struct Conn(Stream);

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl IpcStream for Conn {}
