//! `fo-nativehost` — ブラウザ拡張とデーモンの中継。
//!
//! ブラウザが spawn する短命プロセス。**自分では DB を触らない。**
//! ブラウザは複数プロファイルから同時にこれを起動しうるので、
//! ここから SQLite を開くと書き込みが競合する。DB への書き込み口は
//! デーモン 1 つに集約する（README §9.1）。
//!
//! ```text
//!   拡張 ──(stdio, 長さ前置 JSON)── fo-nativehost ──(IPC)── fo-daemon
//! ```
//!
//! ## 2 つのプロトコルの違い
//!
//! | | 拡張側 | デーモン側 |
//! |---|---|---|
//! | 枠 | 4 バイトのリトルエンディアン長 + 本体 | 行区切り（末尾 `\n`） |
//! | 上限 | ブラウザ → ホストは 4 MB | なし |
//!
//! 変換するのがこのプロセスの仕事の半分。

use std::io::{BufReader, Read, Write};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Chrome / Firefox がホストへ送るメッセージの上限（4 MB）。
/// これを超える長さが来たら、壊れた入力か別のプロトコル。読み進めずに止める。
const MAX_MESSAGE: u32 = 4 * 1024 * 1024;

/// 拡張から来るメッセージ。
///
/// 拡張側の都合に合わせて緩く受ける。`type` が無いものは
/// ダウンロード報告とみなす（拡張の実装差で落ちないように）。
#[derive(Debug, Deserialize)]
struct Incoming {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(flatten)]
    rest: serde_json::Value,
}

/// 拡張へ返すメッセージ。
#[derive(Debug, Serialize)]
struct Outgoing {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Outgoing {
    fn ok(file_id: Option<i64>, verdict: Option<String>) -> Self {
        Self {
            ok: true,
            file_id,
            verdict,
            error: None,
        }
    }
    fn err(msg: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            file_id: None,
            verdict: None,
            error: Some(msg.to_string()),
        }
    }
}

fn main() -> Result<()> {
    // ブラウザは引数にマニフェストのパスと拡張 ID を渡してくる。使わないが、
    // 人が直接起動したときに「これは何か」を伝える。
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "fo-nativehost: ブラウザ拡張から起動される中継プロセスです。\n\
             直接実行するものではありません。`fo host install` で登録してください。"
        );
        return Ok(());
    }

    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout();

    let platform = fo_platform::current();

    loop {
        let msg = match read_framed(&mut reader)? {
            Some(m) => m,
            // 拡張が接続を閉じた（タブが閉じた、ブラウザが終了した）。正常終了。
            None => return Ok(()),
        };

        let reply = handle(platform.as_ref(), &msg);
        write_framed(&mut stdout, &reply)?;
    }
}

fn handle(platform: &dyn fo_platform::Platform, raw: &[u8]) -> Outgoing {
    let incoming: Incoming = match serde_json::from_slice(raw) {
        Ok(v) => v,
        Err(e) => return Outgoing::err(format!("JSON を解釈できません: {e}")),
    };

    match incoming.r#type.as_deref() {
        Some("ping") => Outgoing::ok(None, None),
        // 型が無い / download のときはダウンロード報告として扱う。
        None | Some("download") => forward(platform, incoming.rest),
        Some(other) => Outgoing::err(format!("未知の type: {other}")),
    }
}

/// 報告をデーモンへ中継する。
fn forward(platform: &dyn fo_platform::Platform, payload: serde_json::Value) -> Outgoing {
    let report: fo_ipc::DownloadReport = match serde_json::from_value(payload) {
        Ok(r) => r,
        Err(e) => return Outgoing::err(format!("報告の形式が違います: {e}")),
    };

    let mut stream = match platform.ipc().connect() {
        Ok(s) => s,
        Err(e) => {
            // デーモンが居ないのはよくある状態（まだ起動していない）。
            // 拡張側でユーザーに案内できるよう、理由が分かる文言にする。
            return Outgoing::err(format!(
                "デーモンに接続できません（fo-daemon を起動してください）: {e}"
            ));
        }
    };

    match fo_ipc::round_trip(&mut stream, &fo_ipc::Request::RecordDownload(report)) {
        Ok(fo_ipc::Response::Recorded { file_id, verdict }) => {
            Outgoing::ok(Some(file_id), Some(verdict))
        }
        Ok(other) => Outgoing::err(format!("想定外の応答: {other:?}")),
        Err(e) => Outgoing::err(e),
    }
}

/// 長さ前置のメッセージを 1 件読む。接続が閉じられていれば `None`。
fn read_framed<R: Read>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e).context("長さを読めません"),
    }

    let len = u32::from_ne_bytes(len_buf);
    if len > MAX_MESSAGE {
        bail!("メッセージが大きすぎます: {len} バイト（上限 {MAX_MESSAGE}）");
    }

    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).context("本体を読めません")?;
    Ok(Some(buf))
}

/// 長さ前置のメッセージを 1 件書く。
fn write_framed<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len()).context("応答が大きすぎます")?;
    // ブラウザはネイティブバイトオーダーを期待する（仕様がそう書いている）。
    w.write_all(&len.to_ne_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_framing() {
        let mut buf = Vec::new();
        write_framed(&mut buf, &serde_json::json!({"a": 1})).unwrap();

        // 4 バイトの長さ + 本体
        assert_eq!(buf.len(), 4 + buf.len() - 4);
        let len = u32::from_ne_bytes(buf[..4].try_into().unwrap()) as usize;
        assert_eq!(len, buf.len() - 4);

        let mut r = &buf[..];
        let back = read_framed(&mut r).unwrap().unwrap();
        assert_eq!(back, &buf[4..]);
    }

    #[test]
    fn closed_stream_is_none() {
        let mut r = &b""[..];
        assert!(read_framed(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_length() {
        // 壊れた入力で巨大な確保をしない。
        let mut buf = (MAX_MESSAGE + 1).to_ne_bytes().to_vec();
        buf.extend_from_slice(b"x");
        let mut r = &buf[..];
        assert!(read_framed(&mut r).is_err());
    }

    #[test]
    fn unknown_type_is_reported_not_fatal() {
        let platform = fo_platform::current();
        let out = handle(platform.as_ref(), br#"{"type":"whatever"}"#);
        assert!(!out.ok);
        assert!(out.error.unwrap().contains("whatever"));
    }
}
