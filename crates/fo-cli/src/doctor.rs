//! `fo doctor` — 実行環境の能力を診断する。
//!
//! **能力差を隠さないことが目的。** Windows と Linux は対称ではなく、
//! 特権の有無でも挙動が変わる。ユーザーが「記録されない」と「壊れている」を
//! 区別できないと、このツールは信用されない。
//!
//! 使えない項目には、次に何をすればよいかを必ず添える。

use std::io::Result;

use fo_platform::{Capabilities, Capability, Platform};

pub fn run(platform: &dyn Platform) -> Result<()> {
    let caps = platform.capabilities();
    let paths = platform.paths();

    outln!("Platform          : {}", caps.platform_name);
    line("Stable file ID", &caps.stable_file_id)?;
    line("Reverse lookup", &caps.reverse_lookup)?;

    print_origin_sources(&caps)?;

    line("Change journal", &caps.change_journal)?;
    if !caps.change_journal.is_available() {
        note("既定の差分スキャンで補正します（機能は失われません）")?;
    }

    outln!();
    outln!("Data dir          : {}", paths.data_dir().display());
    outln!("Config dir        : {}", paths.config_dir().display());
    outln!("Database          : {}", paths.database_path().display());
    outln!("IPC               : {}", platform.ipc().endpoint_display());
    outln!(
        "Daemon            : {}",
        if crate::daemon::is_running(platform) {
            "稼働中"
        } else {
            "停止中（自動記録と即時追従は無効）"
        }
    );

    let downloads = paths.default_download_dirs();
    if downloads.is_empty() {
        outln!("Download dirs     : (検出できず — fo scan <path> で明示してください)");
    } else {
        for (i, d) in downloads.iter().enumerate() {
            let label = if i == 0 {
                "Download dirs     :"
            } else {
                "                   "
            };
            outln!("{label} {}", d.display());
        }
    }
    Ok(())
}

fn print_origin_sources(caps: &Capabilities) -> Result<()> {
    if caps.origin_sources.is_empty() {
        return line(
            "Origin metadata",
            &Capability::Unavailable {
                why: "経路なし"
            },
        );
    }

    for (i, (source, cap)) in caps.origin_sources.iter().enumerate() {
        let label = if i == 0 { "Origin metadata" } else { "" };
        outln!("{:<18}: {} {}", label, mark(cap), source.as_str());
        detail(cap)?;
    }

    // OS 固有の注意書きはプラットフォーム層が持つ。
    // ここで OS を判定すると、表示側に OS 知識が漏れる（設計方針 P1）。
    for advice in &caps.advice {
        note(advice)?;
    }
    Ok(())
}

fn line(label: &str, cap: &Capability) -> Result<()> {
    outln!("{label:<18}: {} {}", mark(cap), summary(cap));
    detail(cap)
}

fn mark(cap: &Capability) -> &'static str {
    match cap {
        Capability::Available => "[ok]",
        Capability::NeedsPrivilege { .. } | Capability::OptIn { .. } => "[--]",
        Capability::Unavailable { .. } => "[xx]",
    }
}

fn summary(cap: &Capability) -> &'static str {
    match cap {
        Capability::Available => "利用可能",
        Capability::NeedsPrivilege { .. } => "特権が必要",
        Capability::OptIn { .. } => "無効（オプトイン）",
        Capability::Unavailable { .. } => "利用不可",
    }
}

fn detail(cap: &Capability) -> Result<()> {
    match cap {
        Capability::Available => Ok(()),
        Capability::NeedsPrivilege { how } => note(how),
        Capability::OptIn { how } => note(how),
        Capability::Unavailable { why } => note(why),
    }
}

fn note(text: &str) -> Result<()> {
    outln!("{:<18}  {}", "", text);
    Ok(())
}
