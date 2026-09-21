// File Origin ブラウザ拡張 — ダウンロード完了を検知してネイティブホストへ報告する。
//
// Chrome(MV3) と Firefox(MV2/MV3) で共通。差は `browser` / `chrome` の名前空間だけなので、
// 先頭で吸収する。
//
// ## なぜ onCreated ではなく onChanged を待つのか
//
// ダウンロード開始時点のファイル名は `foo.zip.crdownload` / `foo.zip.part` という
// 一時名で、最終的な保存先はまだ決まっていない（ユーザーが保存ダイアログで
// 変えることもある）。`state === "complete"` になって初めて確定する。
//
// ## 記録するもの
//
// - finalUrl: リダイレクトを追った実際の取得先
// - url     : 最初にクリックされた URL（finalUrl が無い場合の代替）
// - referrer: ダウンロードリンクがあったページ
//
// referrer があると「どのページから落としたか」が分かる。これは後から
// 配布元を辿り直すときに URL 本体より役に立つことが多い。

const api = typeof browser !== "undefined" ? browser : chrome;

/** ネイティブホスト名。fo-platform の HOST_NAME と一致させること。 */
const HOST_NAME = "io.github.file_origin";

/** 直近の結果。ポップアップが読む。 */
let lastStatus = { state: "idle", message: "まだ記録していません" };

/** ダウンロード中の一時ファイル名。報告しても意味がないので弾く。 */
const IN_PROGRESS = [".crdownload", ".part", ".partial", ".download", ".tmp"];

function isInProgress(path) {
  const lower = (path || "").toLowerCase();
  return IN_PROGRESS.some((s) => lower.endsWith(s));
}

/** このブラウザの名前。記録に残して、後からどの経路か分かるようにする。 */
function browserName() {
  if (typeof browser !== "undefined" && typeof chrome === "undefined") return "firefox";
  const ua = navigator.userAgent;
  if (ua.includes("Edg/")) return "edge";
  if (ua.includes("Firefox/")) return "firefox";
  return "chrome";
}

/**
 * ダウンロード 1 件をネイティブホストへ報告する。
 *
 * `sendNativeMessage` は 1 往復で完結する。接続を張り続ける `connectNative` は
 * ホストプロセスを常駐させてしまい、MV3 の Service Worker が寝ると切れるので使わない。
 */
async function report(item) {
  const payload = {
    path: item.filename,
    url: item.finalUrl || item.url || null,
    referrer: item.referrer || null,
    acquired_at: item.endTime ? Math.floor(Date.parse(item.endTime) / 1000) : null,
    mime: item.mime || null,
    bytes: typeof item.fileSize === "number" && item.fileSize > 0 ? item.fileSize : null,
    browser: browserName(),
    profile: null,
  };

  if (!payload.url && !payload.referrer) {
    // URL の無い報告は記録されない（デーモン側で弾かれる）。
    // ここで止めて、無駄な往復とエラー表示を避ける。
    setStatus("skipped", `URL が取れませんでした: ${baseName(payload.path)}`);
    return;
  }

  try {
    const res = await sendNative(payload);
    if (res && res.ok) {
      setStatus("ok", `記録しました: ${baseName(payload.path)}`);
    } else {
      setStatus("error", (res && res.error) || "不明なエラー");
    }
  } catch (e) {
    // ホスト未登録・デーモン停止のどちらもここに来る。
    // 利用者が次に何をすればよいか分かる文言にする。
    setStatus(
      "error",
      `ネイティブホストに接続できません。` +
        `fo host install を実行し、fo-daemon を起動してください。(${e.message || e})`
    );
  }
}

/** Promise と callback の両方式に対応する（Firefox は Promise、Chrome は callback）。 */
function sendNative(payload) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const done = (v) => {
      if (!settled) {
        settled = true;
        resolve(v);
      }
    };
    const fail = (e) => {
      if (!settled) {
        settled = true;
        reject(e);
      }
    };

    try {
      const maybe = api.runtime.sendNativeMessage(HOST_NAME, payload, (res) => {
        const err = api.runtime.lastError;
        if (err) fail(new Error(err.message));
        else done(res);
      });
      // Firefox は Promise を返す。
      if (maybe && typeof maybe.then === "function") {
        maybe.then(done, fail);
      }
    } catch (e) {
      fail(e);
    }
  });
}

function baseName(p) {
  if (!p) return "(不明)";
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

function setStatus(state, message) {
  lastStatus = { state, message, at: Date.now() };
  // MV3 の Service Worker は寝ると変数が消える。ポップアップが後から読めるよう保存する。
  try {
    api.storage.local.set({ lastStatus });
  } catch (_) {
    /* storage 権限が無い構成でも動くようにする */
  }
}

api.downloads.onChanged.addListener((delta) => {
  // 完了した瞬間だけ拾う。
  if (!delta.state || delta.state.current !== "complete") return;

  api.downloads.search({ id: delta.id }, (items) => {
    const item = items && items[0];
    if (!item || !item.filename) return;
    if (isInProgress(item.filename)) return;
    report(item);
  });
});

// ポップアップからの問い合わせ。
api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "getStatus") {
    api.storage.local.get("lastStatus", (v) => {
      sendResponse((v && v.lastStatus) || lastStatus);
    });
    return true; // 非同期で返す
  }
  if (msg && msg.type === "ping") {
    sendNative({ type: "ping" }).then(
      (res) => sendResponse({ ok: !!(res && res.ok), res }),
      (e) => sendResponse({ ok: false, error: e.message || String(e) })
    );
    return true;
  }
  return false;
});
