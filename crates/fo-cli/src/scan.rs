//! `fo scan` — 走査の進捗を集計して表示する。ロジックは `fo_app::scan_dir` にある。

use std::path::Path;

use anyhow::Result;

use fo_app::{scan_dir, ScanEvent, ScanOptions};
use fo_core::Verdict;
use fo_platform::Platform;
use fo_store::Store;

#[derive(Default)]
struct Summary {
    seen: usize,
    added: usize,
    unchanged: usize,
    moved: usize,
    copied: usize,
    updated: usize,
    origins_found: usize,
    errors: usize,
}

pub fn run(platform: &dyn Platform, store: &Store, root: &Path, opts: ScanOptions) -> Result<()> {
    let mut s = Summary::default();

    let root = scan_dir(platform, store, root, opts, &mut |ev| match ev {
        ScanEvent::File {
            verdict,
            os_origins_recorded,
            ..
        } => {
            s.seen += 1;
            s.origins_found += os_origins_recorded;
            match verdict {
                Verdict::New => s.added += 1,
                Verdict::Same { .. } => s.unchanged += 1,
                Verdict::Moved { .. } => s.moved += 1,
                Verdict::Copied { .. } => s.copied += 1,
                Verdict::Updated { .. } => s.updated += 1,
            }
        }
        ScanEvent::Error { .. } => s.errors += 1,
    })?;

    outln!("走査済み: {}", root.display());
    outln!();
    outln!("  検出      : {} ファイル", s.seen);
    outln!("  新規記録  : {}", s.added);
    outln!("  変更なし  : {}", s.unchanged);
    if s.moved > 0 {
        outln!("  移動検出  : {}", s.moved);
    }
    if s.copied > 0 {
        outln!("  コピー検出: {}", s.copied);
    }
    if s.updated > 0 {
        outln!("  内容更新  : {}", s.updated);
    }
    outln!("  入手元取得: {}", s.origins_found);
    if s.errors > 0 {
        outln!(
            "  エラー    : {}（権限不足など。処理は継続しました）",
            s.errors
        );
    }

    // 入手元を読みに行くのは新規・コピーのときだけなので、
    // 新規が 0 件なら「取れなかった」と言うのは筋違い。
    if s.origins_found == 0 && s.added > 0 {
        outln!();
        outln!(
            "新規 {} 件のうち、入手元が取れたものはありませんでした。",
            s.added
        );
        outln!("`fo doctor` でこの環境の取得経路を確認してください。");
    }

    Ok(())
}
