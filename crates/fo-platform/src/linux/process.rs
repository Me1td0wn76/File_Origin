//! Linux のプロセス起動。

use std::os::unix::process::CommandExt;
use std::path::Path;

use crate::Result;

pub fn spawn_background(exe: &Path, args: &[&str]) -> Result<u32> {
    // 自分のプロセスグループから外す。端末から GUI を起動していた場合、
    // Ctrl-C や端末の終了で送られる信号がグループ全体に届くので、
    // そのままだとデーモンまで道連れになる。
    let mut child = crate::background_command(exe, args)
        .process_group(0)
        .spawn()?;
    let pid = child.id();

    // 終了を待つだけのスレッドを残す。待たずに捨てると、デーモンが
    // 終わったあとゾンビとして残る（親の GUI は長生きするので消えない）。
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(pid)
}
