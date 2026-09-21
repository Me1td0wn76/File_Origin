//! `fo search` / `fo where` — 一覧表示。条件の解釈と検索は `fo_app::search` にある。

use anyhow::{bail, Result};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use fo_core::model::{FileStatus, SearchHit, SearchQuery, SortKey, SortOrder};
use fo_store::Store;

pub struct Args {
    pub name: Option<String>,
    pub url: Option<String>,
    pub host: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: usize,
    pub sort: String,
    /// 向きの指定。`None` なら項目ごとの自然な向き。
    ///
    /// bool 1 つ（`--asc` だけ）にすると、名前順は既定が昇順なので
    /// **降順にする手段が無くなる**。向きは 3 状態で持つ。
    pub direction: Option<bool>,
}

pub fn run(store: &Store, args: Args) -> Result<()> {
    let q = SearchQuery {
        name: args.name,
        url: args.url,
        host: args.host,
        since: args.since.as_deref().map(parse_day_start).transpose()?,
        // --until は「その日まで（含む）」と読む方が自然なので、翌日 0 時を未満で使う。
        until: args
            .until
            .as_deref()
            .map(parse_next_day_start)
            .transpose()?,
        sha256: None,
        limit: args.limit,
        sort: parse_sort(&args.sort, args.direction)?,
    };
    let hits = fo_app::search(store, q)?;
    print_hits(&hits, args.limit)?;
    Ok(())
}

pub fn locate(store: &Store, query: &str) -> Result<()> {
    let hits = fo_app::locate(store, query)?;
    if hits.is_empty() {
        outln!("見つかりません: {query}");
        outln!("ファイル名の一部か、SHA-256（64 桁）で指定してください。");
        return Ok(());
    }
    print_hits(&hits, 0)?;
    Ok(())
}

fn print_hits(hits: &[SearchHit], limit: usize) -> Result<()> {
    if hits.is_empty() {
        outln!("該当なし");
        return Ok(());
    }
    for h in hits {
        let status = match h.record.status {
            FileStatus::Present => "",
            FileStatus::Missing => "  [見失い中]",
            FileStatus::Deleted => "  [削除済み]",
        };
        outln!("{}{}", h.record.current_path.display(), status);
        match &h.best_origin {
            Some(o) => {
                let when = o.acquired_at.map(fmt_day).unwrap_or_default();
                outln!(
                    "    [{}] {}{}",
                    o.confidence.as_str(),
                    o.url.as_deref().unwrap_or("(URL なし)"),
                    if when.is_empty() {
                        String::new()
                    } else {
                        format!("  ({when})")
                    }
                );
            }
            None => outln!("    入手元: 記録なし"),
        }
    }
    outln!();
    if limit > 0 && hits.len() >= limit {
        outln!("{} 件（上限）。--limit で増やせます。", hits.len());
    } else {
        outln!("{} 件", hits.len());
    }
    Ok(())
}

/// `--sort` と向きの指定を並び順にする。
///
/// 向きを省いた場合は項目ごとの自然な向き（日時・サイズ・確度は降順、名前は昇順）。
/// どの項目でも既定が降順だと、名前順が Z から始まって使いにくい。
fn parse_sort(key: &str, direction: Option<bool>) -> Result<SortOrder> {
    let Some(key) = SortKey::parse(key) else {
        bail!(
            "並べ替えの項目が違います: {key}（使えるのは {}）",
            SortKey::all()
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        );
    };
    Ok(match direction {
        Some(descending) => SortOrder::new(key, descending),
        None => SortOrder::natural(key),
    })
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    #[test]
    fn direction_defaults_to_natural_per_key() {
        // 名前は昇順、日時・サイズ・確度は降順が自然。
        assert!(!parse_sort("name", None).unwrap().descending);
        assert!(parse_sort("size", None).unwrap().descending);
        assert!(parse_sort("acquired", None).unwrap().descending);
        assert!(parse_sort("confidence", None).unwrap().descending);
    }

    #[test]
    fn direction_can_be_forced_both_ways() {
        // --asc だけだと名前の降順が出せない。両方向を指定できること。
        assert!(parse_sort("name", Some(true)).unwrap().descending);
        assert!(!parse_sort("size", Some(false)).unwrap().descending);
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_sort("bogus", None).is_err());
    }
}

/// `YYYY-MM-DD` をローカル時刻のその日 0 時として Unix 秒に。
fn parse_day_start(s: &str) -> Result<i64> {
    let day = parse_date(s)?;
    local_midnight(day)
}

fn parse_next_day_start(s: &str) -> Result<i64> {
    let day = parse_date(s)?;
    let next = day
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("日付が範囲外です: {s}"))?;
    local_midnight(next)
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        Ok(d) => Ok(d),
        Err(_) => bail!("日付は YYYY-MM-DD で指定してください: {s}"),
    }
}

fn local_midnight(day: NaiveDate) -> Result<i64> {
    let naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("日付が範囲外です"))?;
    // 夏時間の切り替えで 0 時が存在しない日は、存在する最も早い時刻に寄せる。
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.timestamp())
        .ok_or_else(|| anyhow::anyhow!("ローカル時刻に変換できません: {day}"))
}

fn fmt_day(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    DateTime::from_timestamp(ts, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_is_inclusive_of_the_day() {
        let start = parse_day_start("2026-09-21").unwrap();
        let end = parse_next_day_start("2026-09-21").unwrap();
        assert_eq!(end - start, 24 * 60 * 60);
    }

    #[test]
    fn rejects_bad_dates() {
        assert!(parse_date("2026/09/21").is_err());
        assert!(parse_date("yesterday").is_err());
    }
}
