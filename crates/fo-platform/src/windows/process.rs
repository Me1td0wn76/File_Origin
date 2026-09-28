//! Windows のプロセス起動。
//!
//! GUI からデーモンを立ち上げるときに、黒い窓を出さないための細工。
//! 窓を出さないこと自体は `fo-daemon` 側の `windows_subsystem = "windows"` が
//! 担っているが、それはリリースビルドだけの話なので、こちらでも押さえておく。

use std::os::windows::process::CommandExt;
use std::path::Path;

use crate::Result;

/// `CREATE_NO_WINDOW`。子に新しいコンソールを作らせない。
///
/// デバッグビルドの `fo-daemon` はコンソールサブシステムのままなので、
/// これが無いとビルド種別で挙動が変わる。開発中だけ窓が出る、は追いにくい。
///
/// `DETACHED_PROCESS` と併せては使えない（片方が無視される）。
/// こちらを選ぶのは、コンソールを「切り離す」のではなく「作らせない」のが
/// 欲しい挙動だから。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn spawn_background(exe: &Path, args: &[&str]) -> Result<u32> {
    // Windows の子プロセスは既定で親から独立している（Job に入れない限り）。
    // 親の GUI を閉じてもデーモンは動き続ける。
    let child = crate::background_command(exe, args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;

    // `Child` を落としてもプロセスは死なない。閉じるのはハンドルだけ。
    // Unix と違いゾンビも残らないので、待つスレッドは要らない。
    Ok(child.id())
}
