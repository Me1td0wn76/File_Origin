//! デーモンとクライアント（CLI / GUI / Native Messaging ホスト）の間のプロトコル。
//!
//! **行区切り JSON**（1 行 1 メッセージ、UTF-8、末尾 `\n`）。
//! JSON-RPC 2.0 のような汎用性は要らない — 話す相手が自分たちだけなので、
//! 素直な `{"kind": ...}` の方が読みやすく、他言語からも実装しやすい。
//!
//! ## 互換性
//!
//! ブラウザ拡張（M4）は Native Messaging ホスト越しにこれを使う。
//! 拡張とデーモンの更新は同時にできないので、**未知のフィールドは無視する**
//! （serde の既定）。フィールドを消すのではなく `Option` にして残す。

use std::io::{BufRead, BufReader, Write};

use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IPC の入出力: {0}")]
    Io(#[from] std::io::Error),

    #[error("メッセージを解釈できません: {0}")]
    Protocol(#[from] serde_json::Error),

    #[error("接続が閉じられました")]
    Closed,

    #[error("デーモンからのエラー: {0}")]
    Remote(String),
}

/// クライアント → デーモン。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// 生存確認。
    Ping,
    /// 稼働状況を問い合わせる。
    Status,
    /// ダウンロード完了の通知。ブラウザ拡張からの本命の経路（M4）。
    ///
    /// デーモンは `path` のファイルを取り込み、この入手元を `browser_ext` として
    /// 記録する。拡張が報告する情報は OS メタデータより豊富で正確なので、
    /// 確度は `certain`。
    RecordDownload(DownloadReport),
    /// 監視対象を追加して即時スキャンする。
    AddRoot { path: String, recursive: bool },
    /// 監視対象を一覧する。
    ListRoots,
    /// 終了を要求する。
    Shutdown,
}

/// デーモン → クライアント。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Pong,
    Status(DaemonStatus),
    /// 取り込みが成功した。
    Recorded {
        file_id: i64,
        verdict: String,
    },
    Roots {
        paths: Vec<String>,
    },
    Ok,
    Error {
        message: String,
    },
}

/// ブラウザ拡張が報告するダウンロード 1 件。
///
/// フィールド名は WebExtensions の `downloads.DownloadItem` に寄せてある。
/// 拡張側で詰め替える手間を減らし、対応関係を追いやすくするため。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadReport {
    /// 保存先の絶対パス。拡張の `DownloadItem.filename`。
    pub path: String,
    /// リダイレクトを追った最終的な取得先。
    pub url: Option<String>,
    /// ダウンロードリンクがあったページ。
    pub referrer: Option<String>,
    /// 取得完了時刻（Unix 秒）。分からなければ `None`。
    pub acquired_at: Option<i64>,
    pub mime: Option<String>,
    pub bytes: Option<u64>,
    /// 報告元のブラウザ名（`chrome` / `firefox` など）。
    pub browser: Option<String>,
    /// ブラウザのプロファイル識別子。複数プロファイルの区別に使う。
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    /// 監視中のディレクトリ。
    pub roots: Vec<String>,
    /// 起動してからの秒数。
    pub uptime_secs: u64,
    /// 記録済みファイル数。
    pub files: i64,
    /// 監視が動いているか。`false` なら差分スキャンのみ。
    pub watching: bool,
    /// ログファイルの場所。
    ///
    /// コンソール窓を出さなくなったので、**どこを見れば動きが分かるか**を
    /// 教える必要がある。古いデーモンは送ってこないので `Option` かつ `default`。
    #[serde(default)]
    pub log_path: Option<String>,
}

/// メッセージを 1 件書く。
pub fn write_message<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<()> {
    serde_json::to_writer(&mut *w, msg)?;
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

/// メッセージを 1 件読む。接続が閉じられていれば `Error::Closed`。
pub fn read_message<R: BufRead, T: for<'de> Deserialize<'de>>(r: &mut R) -> Result<T> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(Error::Closed);
    }
    Ok(serde_json::from_str(line.trim_end())?)
}

/// 1 往復する。CLI と GUI はこれだけ使えばよい。
pub fn round_trip<S: std::io::Read + Write>(stream: &mut S, req: &Request) -> Result<Response> {
    write_message(stream, req)?;
    let mut reader = BufReader::new(stream);
    let res: Response = read_message(&mut reader)?;
    if let Response::Error { message } = res {
        return Err(Error::Remote(message));
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_bytes() {
        let req = Request::RecordDownload(DownloadReport {
            path: r"C:\dl\a.zip".into(),
            url: Some("https://example.com/a.zip".into()),
            referrer: Some("https://example.com/".into()),
            acquired_at: Some(1_700_000_000),
            mime: Some("application/zip".into()),
            bytes: Some(123),
            browser: Some("chrome".into()),
            profile: None,
        });

        let mut buf = Vec::new();
        write_message(&mut buf, &req).unwrap();
        // 1 行で終わること（行区切りプロトコルの前提）
        assert_eq!(buf.iter().filter(|b| **b == b'\n').count(), 1);

        let mut r = std::io::BufReader::new(&buf[..]);
        let back: Request = read_message(&mut r).unwrap();
        match back {
            Request::RecordDownload(d) => {
                assert_eq!(d.url.as_deref(), Some("https://example.com/a.zip"));
                assert_eq!(d.browser.as_deref(), Some("chrome"));
            }
            other => panic!("想定外: {other:?}"),
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // 拡張が先に更新されてもデーモンが壊れないこと。
        let line = r#"{"kind":"record_download","path":"/a","url":null,"future_field":1}"#;
        let mut r = std::io::BufReader::new(line.as_bytes());
        let req: Request = read_message(&mut r).unwrap();
        assert!(matches!(req, Request::RecordDownload(_)));
    }

    #[test]
    fn closed_connection_is_distinguishable() {
        let mut r = std::io::BufReader::new(&b""[..]);
        let err = read_message::<_, Request>(&mut r).unwrap_err();
        assert!(matches!(err, Error::Closed));
    }
}
