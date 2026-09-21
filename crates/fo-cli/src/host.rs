//! `fo host` — Native Messaging ホストの登録。
//!
//! ブラウザ拡張がデーモンに話しかけるには、ブラウザ側に
//! 「この実行ファイルを、この拡張に対して起動してよい」と教える必要がある。
//! その設置を代行する。

use anyhow::{bail, Context, Result};
// NativeHostInstaller は import しない。`&dyn NativeHostInstaller` への
// メソッド呼び出しに trait のスコープ入りは要らないため。
use fo_platform::{Browser, HostManifest, Platform};

/// 中継プロセスの場所を推測する。
///
/// 同じディレクトリに置かれている前提。配布形態（MSI / .deb / cargo build）に
/// かかわらず `fo` と `fo-nativehost` は隣り合うので、これで足りる。
fn default_host_exe() -> Result<std::path::PathBuf> {
    let me = std::env::current_exe().context("自分の場所が分かりません")?;
    let dir = me.parent().context("実行ファイルの親が取れません")?;
    let name = if std::env::consts::EXE_SUFFIX.is_empty() {
        "fo-nativehost".to_string()
    } else {
        format!("fo-nativehost{}", std::env::consts::EXE_SUFFIX)
    };
    Ok(dir.join(name))
}

pub fn install(
    platform: &dyn Platform,
    browsers: &[String],
    extension_ids: &[String],
    exe: Option<std::path::PathBuf>,
) -> Result<()> {
    if extension_ids.is_empty() {
        bail!(
            "拡張 ID を --extension-id で指定してください。\n\
             空のまま登録すると、どの拡張からでも接続できてしまいます。\n\
             ID はブラウザの拡張機能ページ（chrome://extensions）で確認できます。"
        );
    }

    let exe_path = match exe {
        Some(p) => p,
        None => default_host_exe()?,
    };
    if !exe_path.exists() {
        bail!(
            "中継プロセスが見つかりません: {}\n\
             --exe で明示するか、fo-nativehost を fo と同じ場所に置いてください。",
            exe_path.display()
        );
    }

    let targets = resolve_browsers(browsers)?;
    let manifest = HostManifest {
        exe_path,
        allowed: extension_ids.to_vec(),
    };

    for browser in targets {
        let installed = platform
            .host_installer()
            .install(browser, &manifest)
            .with_context(|| format!("{} への登録に失敗しました", browser.as_str()))?;
        outln!("{} : 登録しました", browser.as_str());
        outln!("  マニフェスト : {}", installed.manifest_path.display());
        if let Some(key) = &installed.registry_key {
            outln!("  レジストリ   : {key}");
        }
    }

    outln!();
    outln!("ブラウザを再起動すると有効になります。");
    outln!("記録には fo-daemon の起動も必要です。");
    Ok(())
}

pub fn uninstall(platform: &dyn Platform, browsers: &[String]) -> Result<()> {
    for browser in resolve_browsers(browsers)? {
        platform.host_installer().uninstall(browser)?;
        outln!("{} : 登録を解除しました", browser.as_str());
    }
    Ok(())
}

pub fn status(platform: &dyn Platform) -> Result<()> {
    let inst = platform.host_installer();
    for browser in Browser::all() {
        let mark = if inst.is_installed(*browser) {
            "○"
        } else {
            "×"
        };
        outln!(
            "{mark} {:<9} {}",
            browser.as_str(),
            inst.manifest_path(*browser).display()
        );
    }
    outln!();
    outln!("× は未登録です。`fo host install --browser <名前> --extension-id <ID>` で登録します。");
    Ok(())
}

/// 引数のブラウザ名を解決する。空なら全部。
fn resolve_browsers(names: &[String]) -> Result<Vec<Browser>> {
    if names.is_empty() {
        return Ok(Browser::all().to_vec());
    }
    let mut out = Vec::new();
    for n in names {
        match Browser::parse(n) {
            Some(b) => out.push(b),
            None => bail!(
                "未知のブラウザ: {n}（使えるのは {}）",
                Browser::all()
                    .iter()
                    .map(|b| b.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
        }
    }
    Ok(out)
}
