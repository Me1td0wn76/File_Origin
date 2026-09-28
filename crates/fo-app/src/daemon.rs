//! 常駐デーモンの起動・停止・生死確認。
//!
//! **CLI と GUI の両方がここを呼ぶ**（設計方針 P2）。
//! 「既に動いていたらどうするか」「いつ起動できたと見なすか」は
//! 判断が要る部分なので、UI ごとに書くと必ず食い違う。
//!
//! ここはプロセスを起こすだけで、デーモンが何を監視するかには関与しない。
//! 監視対象も DB も、デーモンが自分で `Platform` から同じ既定値を引く。
//! 呼び出し側から渡すと、GUI と CLI とデーモンで既定が三重管理になる。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use fo_platform::Platform;

use crate::{Error, Result};

/// デーモンの実行ファイル名。拡張子は OS が決める。
const DAEMON: &str = "fo-daemon";

/// 起動・停止を見届ける上限。
///
/// デーモンは IPC を張った直後に受け付けを始めるので、普通は 1 回目か 2 回目の
/// 問い合わせで返る。長めに取ってあるのは、初回スキャンや DB の
/// オープンと競合したときの余裕。
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

/// デーモンが応答するか。
///
/// 接続できるだけでは足りない。`bind` 済みで受け付け前という状態がありうるので、
/// 実際に 1 往復させて確かめる。
pub fn is_running(platform: &dyn Platform) -> bool {
    platform
        .ipc()
        .connect()
        .ok()
        .and_then(|mut s| fo_ipc::round_trip(&mut s, &fo_ipc::Request::Ping).ok())
        .is_some()
}

/// 自分の隣にある実行ファイルを指す。
///
/// `fo` / `fo-gui` / `fo-daemon` / `fo-nativehost` は同じ場所に並ぶ前提。
/// インストール後もそうだし、`cargo build` の出力（`target/debug`）でもそう。
pub fn sibling_exe(name: &str) -> Result<PathBuf> {
    let me = std::env::current_exe()?;
    let dir = me
        .parent()
        .ok_or_else(|| std::io::Error::other(format!("実行ファイルの親が取れません: {me:?}")))?;
    Ok(dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

/// デーモンを起動する。
///
/// 戻り値は **この呼び出しで起動したか**。既に動いていたら `false` で、
/// これは失敗ではない。ボタンを押した人にとっては「動いている」という
/// 結果が同じなので、エラーにして驚かせる理由が無い。
pub fn start(platform: &dyn Platform) -> Result<bool> {
    if is_running(platform) {
        return Ok(false);
    }

    let exe = sibling_exe(DAEMON)?;
    if !exe.exists() {
        return Err(Error::DaemonNotFound(exe));
    }

    platform.spawn_background(&exe, &[])?;

    // 起こしただけでは「動いている」と言えない。応答を待つ。
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    while Instant::now() < deadline {
        if is_running(platform) {
            return Ok(true);
        }
        std::thread::sleep(POLL);
    }

    // 起動はしたが応答が無い。監視対象が無い・DB が開けないなどで
    // すぐ終了した可能性が高い。理由はログにある。
    Err(Error::DaemonNoResponse(platform.paths().log_dir()))
}

/// デーモンを止める。
///
/// 戻り値は **停止まで見届けられたか**。動いていなければ `false`。
/// 要求は通ったが終了が遅い場合も `false` を返す — 「止めた」と言い切って
/// 直後の表示が「稼働中」になるより、待ってもらうほうがましなので。
pub fn stop(platform: &dyn Platform) -> Result<bool> {
    let Ok(mut stream) = platform.ipc().connect() else {
        return Ok(false);
    };
    fo_ipc::round_trip(&mut stream, &fo_ipc::Request::Shutdown)?;

    let deadline = Instant::now() + SETTLE_TIMEOUT;
    while Instant::now() < deadline {
        if !is_running(platform) {
            return Ok(true);
        }
        std::thread::sleep(POLL);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_is_next_to_me() {
        let me = std::env::current_exe().unwrap();
        let got = sibling_exe("fo-daemon").unwrap();
        assert_eq!(got.parent(), me.parent());
        assert!(got
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("fo-daemon"));
    }

    #[test]
    fn sibling_carries_the_platform_suffix() {
        let got = sibling_exe("fo-daemon").unwrap();
        let name = got.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, format!("fo-daemon{}", std::env::consts::EXE_SUFFIX));
    }

    #[test]
    fn mock_refuses_to_spawn() {
        // モックでうっかり本物のデーモンが起きないこと。
        let p = fo_platform::mock::MockPlatform::new();
        let err = p.spawn_background(std::path::Path::new("fo-daemon"), &[]);
        assert!(matches!(err, Err(fo_platform::Error::Unsupported(_))));
    }
}
