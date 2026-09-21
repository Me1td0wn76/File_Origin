//! `fo show` — 来歴を表示する。データは `fo_app::describe` が集める。

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local};
use fo_app::{describe, Description};
use fo_core::model::Origin;
use fo_platform::Platform;
use fo_store::Store;

pub fn run(platform: &dyn Platform, store: &Store, path: &Path) -> Result<()> {
    let d = match describe(platform, store, path) {
        Ok(d) => d,
        Err(fo_app::Error::NotRecorded(p)) => {
            println!("記録がありません: {}", p.display());
            println!("`fo scan <dir>` で取り込むか、`fo add <path> --url <url>` で登録してください。");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    print(&d);
    Ok(())
}

fn print(d: &Description) {
    let r = &d.record;
    println!("パス       : {}", r.current_path.display());
    println!("サイズ     : {} バイト", r.size);
    println!(
        "SHA-256    : {}",
        r.sha256.as_ref().map(|d| d.as_str()).unwrap_or("(未計算)")
    );
    println!("識別子     : {}", r.stable_id);
    println!("更新日時   : {}", fmt_time(r.mtime));

    // コピー元の系譜。コピーでなければ出さない。
    if let Some(parent) = d.lineage.first() {
        println!("コピー元   : {}", parent.current_path.display());
        for ancestor in d.lineage.iter().skip(1) {
            println!("             ← {}", ancestor.current_path.display());
        }
    }

    // パス履歴。1 件しか無ければ「移動していない」ので出さない。
    if d.paths.len() > 1 {
        println!();
        println!("パス履歴 ({} 件、新しい順):", d.paths.len());
        for p in &d.paths {
            let mark = if p.is_current { "現在" } else { "    " };
            println!("  {} {}  {}", fmt_time(p.observed_at), mark, p.path.display());
        }
    }

    println!();
    if d.origins.is_empty() {
        println!("入手元     : このファイル自身には記録なし");
    } else {
        println!("入手元 ({} 件、確度の高い順):", d.origins.len());
        for o in &d.origins {
            print_origin(o);
        }
    }

    // 祖先の入手元。自身に記録が無いコピーでも、ここで出所が分かる。
    if !d.inherited_origins.is_empty() {
        println!();
        println!("コピー元の入手元 ({} 件):", d.inherited_origins.len());
        for (_, o) in &d.inherited_origins {
            print_origin(o);
        }
    }
}

fn print_origin(o: &Origin) {
    println!(
        "  [{}] {}",
        o.confidence.as_str(),
        o.url.as_deref().unwrap_or("(URL なし)")
    );
    if let Some(r) = &o.referrer_url {
        println!("      参照元  : {r}");
    }
    println!("      経路    : {}", o.source.as_str());
    if let Some(t) = o.acquired_at {
        println!("      取得日時: {}", fmt_time(t));
    }
}

/// Unix 秒をローカル時刻で `YYYY-MM-DD HH:MM` に。
/// 0（不明）は空欄にして、1970 年と誤読させない。
fn fmt_time(ts: i64) -> String {
    if ts <= 0 {
        return "----------  --:--".to_string();
    }
    DateTime::from_timestamp(ts, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
