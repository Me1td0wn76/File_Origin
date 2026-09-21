//! `fo daemon` — 常駐サービスとの対話。
//!
//! CLI 自身はデーモンを必要としない（DB を直接読み書きできる）。
//! デーモンは「ブラウザからの報告を受ける」「監視して即時追従する」ために要る。

use anyhow::{Context, Result};
use fo_ipc::{Request, Response};
use fo_platform::Platform;

/// デーモンが動いているか。`fo doctor` と `status` が使う。
pub fn is_running(platform: &dyn Platform) -> bool {
    platform
        .ipc()
        .connect()
        .ok()
        .and_then(|mut s| fo_ipc::round_trip(&mut s, &Request::Ping).ok())
        .is_some()
}

pub fn status(platform: &dyn Platform) -> Result<()> {
    let Ok(mut stream) = platform.ipc().connect() else {
        outln!("デーモン : 停止中");
        outln!("接続先   : {}", platform.ipc().endpoint_display());
        outln!();
        outln!(
            "`fo-daemon` を起動すると、ダウンロードの自動記録と移動の即時追従が有効になります。"
        );
        return Ok(());
    };

    match fo_ipc::round_trip(&mut stream, &Request::Status)? {
        Response::Status(s) => {
            outln!("デーモン : 稼働中 (v{})", s.version);
            outln!("接続先   : {}", platform.ipc().endpoint_display());
            outln!("稼働時間 : {}", human_duration(s.uptime_secs));
            outln!("記録数   : {} ファイル", s.files);
            outln!("監視     : {}", if s.watching { "有効" } else { "無効" });
            for (i, r) in s.roots.iter().enumerate() {
                outln!(
                    "{} {}",
                    if i == 0 {
                        "対象     :"
                    } else {
                        "           "
                    },
                    r
                );
            }
        }
        other => outln!("想定外の応答: {other:?}"),
    }
    Ok(())
}

pub fn stop(platform: &dyn Platform) -> Result<()> {
    let mut stream = platform
        .ipc()
        .connect()
        .context("デーモンに接続できません（動いていない可能性があります）")?;
    fo_ipc::round_trip(&mut stream, &Request::Shutdown)?;
    outln!("停止を要求しました。");
    Ok(())
}

/// 生の JSON を 1 件送って応答を出す。
///
/// 拡張やホストを実装するときの動作確認用。デーモンのプロトコルを
/// ブラウザ抜きで叩けないと、M4 の切り分けができない。
pub fn send_raw(platform: &dyn Platform, line: &str) -> Result<()> {
    use std::io::{BufReader, Write};

    let mut stream = platform
        .ipc()
        .connect()
        .context("デーモンに接続できません")?;
    stream.write_all(line.trim_end().as_bytes())?;
    stream.write_all(
        b"
",
    )?;
    stream.flush()?;

    let mut reader = BufReader::new(&mut stream);
    let res: serde_json::Value = fo_ipc::read_message(&mut reader)?;
    outln!("{}", serde_json::to_string_pretty(&res)?);
    Ok(())
}

pub fn ping(platform: &dyn Platform) -> Result<()> {
    let mut stream = platform
        .ipc()
        .connect()
        .context("デーモンに接続できません")?;
    match fo_ipc::round_trip(&mut stream, &Request::Ping)? {
        Response::Pong => outln!("応答あり"),
        other => outln!("想定外の応答: {other:?}"),
    }
    Ok(())
}

fn human_duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3_600, (secs % 3_600) / 60);
    if d > 0 {
        format!("{d}日 {h}時間")
    } else if h > 0 {
        format!("{h}時間 {m}分")
    } else if m > 0 {
        format!("{m}分")
    } else {
        format!("{secs}秒")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(human_duration(30), "30秒");
        assert_eq!(human_duration(90), "1分");
        assert_eq!(human_duration(3_700), "1時間 1分");
        assert_eq!(human_duration(90_000), "1日 1時間");
    }
}
