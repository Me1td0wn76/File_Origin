//! File Origin のドメイン層。
//!
//! **このクレートは OS を知らない。** OS 固有の処理はすべて
//! `fo-platform` の trait 越しに呼ぶ（設計方針 P1 / ADR-0003）。
//! おかげでここのロジックは `fo-platform::mock` だけでテストでき、
//! Windows の開発機で Linux 相当の挙動も検証できる。

pub mod hash;
pub mod identity;
pub mod model;

pub use identity::{classify, Observed, Verdict};
pub use model::{Confidence, Digest, FileRecord, FileStatus, Origin, OriginSource, PathEntry};
