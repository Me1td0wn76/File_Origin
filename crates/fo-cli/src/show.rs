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
            outln!("記録がありません: {}", p.display());
            outln!(
                "`fo scan <dir>` で取り込むか、`fo add <path> --url <url>` で登録してください。"
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    print(&d)
}

fn print(d: &Description) -> Result<()> {
    let r = &d.record;
    outln!("パス       : {}", r.current_path.display());
    outln!("サイズ     : {} バイト", r.size);
    outln!(
        "SHA-256    : {}",
        r.sha256.as_ref().map(|d| d.as_str()).unwrap_or("(未計算)")
    );
    outln!("識別子     : {}", r.stable_id);
    outln!("更新日時   : {}", fmt_time(r.mtime));

    // コピー元の系譜。コピーでなければ出さない。
    if let Some(parent) = d.lineage.first() {
        outln!("コピー元   : {}", parent.current_path.display());
        for ancestor in d.lineage.iter().skip(1) {
            outln!("             ← {}", ancestor.current_path.display());
        }
    }

    // パス履歴。1 件しか無ければ「移動していない」ので出さない。
    if d.paths.len() > 1 {
        outln!();
        outln!("パス履歴 ({} 件、新しい順):", d.paths.len());
        for p in &d.paths {
            let mark = if p.is_current { "現在" } else { "    " };
            outln!(
                "  {} {}  {}",
                fmt_time(p.observed_at),
                mark,
                p.path.display()
            );
        }
    }

    outln!();
    if d.origins.is_empty() {
        outln!("入手元     : このファイル自身には記録なし");
    } else {
        outln!("入手元 ({} 件、確度の高い順):", d.origins.len());
        for o in &d.origins {
            print_origin(o)?;
        }
    }

    // 祖先の入手元。自身に記録が無いコピーでも、ここで出所が分かる。
    if !d.inherited_origins.is_empty() {
        outln!();
        outln!("コピー元の入手元 ({} 件):", d.inherited_origins.len());
        for (_, o) in &d.inherited_origins {
            print_origin(o)?;
        }
    }
    Ok(())
}

fn print_origin(o: &Origin) -> Result<()> {
    outln!(
        "  [{}] {}",
        o.confidence.as_str(),
        o.url.as_deref().unwrap_or("(URL なし)")
    );
    if let Some(r) = &o.referrer_url {
        outln!("      参照元  : {r}");
    }
    outln!("      経路    : {}", o.source.as_str());
    if let Some(t) = o.acquired_at {
        outln!("      取得日時: {}", fmt_time(t));
    }
    Ok(())
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
