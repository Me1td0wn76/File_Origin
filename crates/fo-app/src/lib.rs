//! アプリケーション層。**CLI と GUI が共有するユースケース。**
//!
//! `fo-core`（ドメイン）・`fo-store`（永続化）・`fo-platform`（OS）を束ねて、
//! 「ファイルを取り込む」「来歴を引く」「入手元を手で足す」といった
//! 1 操作単位の処理を提供する。
//!
//! ここに置く理由: 設計方針 P2「UI はコアの上の薄い層」。
//! これらを CLI に書くと GUI（M5）で同じものをもう一度書くことになり、
//! 2 つの UI で挙動が食い違う。
//!
//! なお README の当初案は `fo-core::usecase` だったが、`fo-store` が `fo-core` に
//! 依存しているため `fo-core` からストアは呼べない。依存方向を守るには
//! 両方の上に載る別クレートが要る（Decision Log D11）。

pub mod describe;
pub mod ingest;
pub mod manual;
pub mod scan;

pub use describe::{describe, Description};
pub use ingest::{ingest_file, IngestOptions, Ingested};
pub use manual::add_manual_origin;
pub use scan::{scan_dir, ScanEvent, ScanOptions};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Platform(#[from] fo_platform::Error),

    #[error(transparent)]
    Store(#[from] fo_store::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("パスを正規化できない {path}: {source}")]
    Canonicalize {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("記録がありません: {0}")]
    NotRecorded(std::path::PathBuf),
}
