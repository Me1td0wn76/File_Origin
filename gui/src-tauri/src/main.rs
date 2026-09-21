//! `fo-gui` — デスクトップ GUI。
//!
//! **CLI と同じ `fo-app` の上に載る薄い層**（設計方針 P2）。
//! 検索も取り込みも来歴の組み立ても `fo-app` の関数を呼ぶだけで、
//! ここにロジックは置かない。置くと CLI と挙動が食い違う。
//!
//! ## DB の持ち方
//!
//! GUI は DB を**自分で開く**。デーモン経由にしないのは、デーモンが
//! 動いていなくても閲覧・検索はできるべきだから（README §11 の
//! 「デーモン不在時は直接オープン」）。書き込みが競合する操作
//! （取り込み）はデーモンが居ればそちらへ回す、という余地は残してある。

// リリースビルドでコンソールウィンドウを出さない。
// 付けないと GUI の裏に黒い窓が残り、「何か変なものが起動した」と見える。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

/// tauri.conf.json の値と揃えること。設定が効かない環境での保険に使う。
const WINDOW_W: f64 = 1100.0;
const WINDOW_H: f64 = 720.0;

use fo_core::model::{FileStatus, SearchQuery, SortKey, SortOrder};
use fo_platform::Platform;
use fo_store::Store;
use serde::Serialize;
use tauri::{Manager, State};

/// アプリ全体で共有する状態。
struct App {
    platform: Box<dyn Platform>,
    store: Mutex<Store>,
}

// --- フロントエンドに渡す型 -------------------------------------------------
// `fo-core` の型をそのまま serde で送らないのは、ドメイン型に
// GUI 都合の derive を足したくないため。ここで表示用に詰め替える。

#[derive(Serialize)]
struct HitDto {
    id: i64,
    path: String,
    name: String,
    size: u64,
    status: String,
    /// 確度の高い入手元。無ければ null。
    url: Option<String>,
    confidence: Option<String>,
    source: Option<String>,
    acquired_at: Option<i64>,
}

#[derive(Serialize)]
struct OriginDto {
    url: Option<String>,
    referrer_url: Option<String>,
    source: String,
    confidence: String,
    acquired_at: Option<i64>,
    browser: Option<String>,
    /// 祖先（コピー元）から継承したものか。
    inherited: bool,
}

#[derive(Serialize)]
struct PathDto {
    path: String,
    is_current: bool,
    observed_at: i64,
}

#[derive(Serialize)]
struct DetailDto {
    id: i64,
    path: String,
    size: u64,
    sha256: Option<String>,
    stable_id: String,
    mtime: i64,
    status: String,
    origins: Vec<OriginDto>,
    paths: Vec<PathDto>,
    /// コピー元の現在地。コピーでなければ空。
    lineage: Vec<String>,
}

/// 並べ替えの選択肢。ラベルの二重定義を避けるため、一覧は Rust 側が持つ。
#[derive(Serialize)]
struct SortOptionDto {
    key: String,
    /// そのキーの自然な向き（true なら降順）。
    default_descending: bool,
}

#[derive(Serialize)]
struct StatusDto {
    files: i64,
    db_path: String,
    platform: String,
    daemon_running: bool,
    ipc_endpoint: String,
    download_dirs: Vec<String>,
    /// OS メタデータ経路についての注意書き（Linux の xattr など）。
    advice: Vec<String>,
}

fn status_str(s: FileStatus) -> &'static str {
    match s {
        FileStatus::Present => "present",
        FileStatus::Missing => "missing",
        FileStatus::Deleted => "deleted",
    }
}

fn base_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

// --- コマンド ---------------------------------------------------------------

#[tauri::command]
fn search(
    state: State<'_, App>,
    name: Option<String>,
    url: Option<String>,
    host: Option<String>,
    limit: Option<usize>,
    sort: Option<String>,
    descending: Option<bool>,
) -> Result<Vec<HitDto>, String> {
    let empty = |s: &Option<String>| s.as_deref().map(str::trim).unwrap_or("").is_empty();

    // 未知のキーが来ても検索自体は成立させる。画面が空になるより、
    // 既定の並びで結果が出るほうがましなので、ここでは弾かない。
    let key = sort.as_deref().and_then(SortKey::parse).unwrap_or_default();
    let order = match descending {
        Some(d) => SortOrder::new(key, d),
        None => SortOrder::natural(key),
    };

    let q = SearchQuery {
        name: if empty(&name) { None } else { name },
        url: if empty(&url) { None } else { url },
        host: if empty(&host) { None } else { host },
        limit: limit.unwrap_or(200),
        sort: order,
        ..Default::default()
    };

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let hits = fo_app::search(&store, q).map_err(|e| e.to_string())?;

    Ok(hits
        .into_iter()
        .map(|h| HitDto {
            id: h.record.id,
            name: base_name(&h.record.current_path),
            path: h.record.current_path.to_string_lossy().into_owned(),
            size: h.record.size,
            status: status_str(h.record.status).to_string(),
            url: h.best_origin.as_ref().and_then(|o| o.url.clone()),
            confidence: h
                .best_origin
                .as_ref()
                .map(|o| o.confidence.as_str().to_string()),
            source: h
                .best_origin
                .as_ref()
                .map(|o| o.source.as_str().to_string()),
            acquired_at: h.best_origin.as_ref().and_then(|o| o.acquired_at),
        })
        .collect())
}

#[tauri::command]
fn detail(state: State<'_, App>, id: i64) -> Result<DetailDto, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let record = store
        .get_file(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("記録が見つかりません: id={id}"))?;

    let d = fo_app::describe::describe_record(&store, record).map_err(|e| e.to_string())?;

    let mut origins: Vec<OriginDto> = d
        .origins
        .iter()
        .map(|o| OriginDto {
            url: o.url.clone(),
            referrer_url: o.referrer_url.clone(),
            source: o.source.as_str().to_string(),
            confidence: o.confidence.as_str().to_string(),
            acquired_at: o.acquired_at,
            browser: o.browser.clone(),
            inherited: false,
        })
        .collect();
    // コピー元から継承した入手元も見せる。コピーには自身の記録が
    // 無いことが多く、これが無いと「出所不明」に見えてしまう。
    origins.extend(d.inherited_origins.iter().map(|(_, o)| OriginDto {
        url: o.url.clone(),
        referrer_url: o.referrer_url.clone(),
        source: o.source.as_str().to_string(),
        confidence: o.confidence.as_str().to_string(),
        acquired_at: o.acquired_at,
        browser: o.browser.clone(),
        inherited: true,
    }));

    Ok(DetailDto {
        id: d.record.id,
        path: d.record.current_path.to_string_lossy().into_owned(),
        size: d.record.size,
        sha256: d.record.sha256.as_ref().map(|x| x.as_str().to_string()),
        stable_id: d.record.stable_id.to_string(),
        mtime: d.record.mtime,
        status: status_str(d.record.status).to_string(),
        origins,
        paths: d
            .paths
            .iter()
            .map(|p| PathDto {
                path: p.path.to_string_lossy().into_owned(),
                is_current: p.is_current,
                observed_at: p.observed_at,
            })
            .collect(),
        lineage: d
            .lineage
            .iter()
            .map(|r| r.current_path.to_string_lossy().into_owned())
            .collect(),
    })
}

#[tauri::command]
fn status(state: State<'_, App>) -> Result<StatusDto, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let caps = state.platform.capabilities();
    let paths = state.platform.paths();

    // デーモンが居るかは ping で確かめる。居なくても GUI は動く。
    let daemon_running = state
        .platform
        .ipc()
        .connect()
        .ok()
        .and_then(|mut s| fo_ipc::round_trip(&mut s, &fo_ipc::Request::Ping).ok())
        .is_some();

    Ok(StatusDto {
        files: store.count_files().map_err(|e| e.to_string())?,
        db_path: paths.database_path().to_string_lossy().into_owned(),
        platform: caps.platform_name,
        daemon_running,
        ipc_endpoint: state.platform.ipc().endpoint_display(),
        download_dirs: paths
            .default_download_dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        advice: caps.advice.iter().map(|s| s.to_string()).collect(),
    })
}

/// 並べ替えに使える項目の一覧。
#[tauri::command]
fn sort_options() -> Vec<SortOptionDto> {
    SortKey::all()
        .iter()
        .map(|k| SortOptionDto {
            key: k.as_str().to_string(),
            default_descending: k.default_descending(),
        })
        .collect()
}

/// ディレクトリを走査して取り込む。
#[tauri::command]
fn scan(state: State<'_, App>, path: String, hash: bool) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let (mut seen, mut added, mut moved, mut origins) = (0, 0, 0, 0);
    let opts = fo_app::ScanOptions {
        recursive: true,
        ingest: fo_app::IngestOptions { hash },
    };
    fo_app::scan_dir(
        state.platform.as_ref(),
        &store,
        std::path::Path::new(&path),
        opts,
        &mut |ev| {
            if let fo_app::ScanEvent::File {
                verdict,
                os_origins_recorded,
                path_changed,
                ..
            } = ev
            {
                seen += 1;
                origins += os_origins_recorded;
                match verdict {
                    fo_core::Verdict::New => added += 1,
                    fo_core::Verdict::Moved { .. } => moved += 1,
                    fo_core::Verdict::Same { .. } if path_changed => moved += 1,
                    _ => {}
                }
            }
        },
    )
    .map_err(|e| e.to_string())?;

    Ok(format!(
        "{seen} ファイル / 新規 {added} / 移動 {moved} / 入手元 {origins}"
    ))
}

/// 入手元を手で登録する。
#[tauri::command]
fn add_origin(
    state: State<'_, App>,
    path: String,
    url: String,
    referrer: Option<String>,
) -> Result<i64, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let (ingested, _) = fo_app::add_manual_origin(
        state.platform.as_ref(),
        &store,
        std::path::Path::new(&path),
        &url,
        referrer.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    Ok(ingested.file_id)
}

fn main() {
    let platform = fo_platform::current();
    let db_path = platform.paths().database_path();
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            // ここで落ちると何も出ないまま終わる。理由を残す。
            eprintln!("DB を開けません {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                // 環境によっては tauri.conf.json の width/height が反映されず、
                // 極端に小さいウィンドウで開くことがある。実測して、
                // 明らかにおかしければ設定値に合わせ直す。
                // 起動直後に中身が読めない窓が出るのは致命的な第一印象になる。
                let before = w.outer_size().ok();
                let scale = w.scale_factor().unwrap_or(1.0);
                let too_small = before.is_some_and(|s| s.width < 400 || s.height < 300);
                if too_small {
                    eprintln!(
                        "ウィンドウが小さすぎます（{:?}, scale={scale}）。設定値に合わせ直します。",
                        before
                    );
                    let _ = w.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        WINDOW_W, WINDOW_H,
                    )));
                    let _ = w.center();
                }
            }
            Ok(())
        })
        .manage(App {
            platform,
            store: Mutex::new(store),
        })
        .invoke_handler(tauri::generate_handler![
            search,
            detail,
            status,
            scan,
            add_origin,
            sort_options
        ])
        .run(tauri::generate_context!())
        .expect("GUI を起動できません");
}
