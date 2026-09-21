// File Origin GUI のフロントエンド。
//
// バンドラを使わない素の JS。理由は 2 つ:
//  - この画面の規模（検索・一覧・詳細）ならフレームワークの利得が無い
//  - npm を挟まない分、`cargo tauri build` だけでビルドが完結する
// 規模が増えたら D5 を見直す。

const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);
const els = {
  q: $("q"), qhost: $("qhost"), results: $("results"), count: $("count"),
  detail: $("detail"), toast: $("toast"),
  sortKey: $("sort-key"), sortDir: $("sort-dir"),
};

let selectedId = null;

/** 並び順。選択は localStorage に覚えさせる。 */
let sort = { key: "first-seen", descending: true };

/** 並べ替えの項目の表示名。キーの一覧は Rust 側（sort_options）が持つ。 */
const SORT_LABEL = {
  "first-seen": "取り込み順",
  acquired: "取得日時",
  name: "ファイル名",
  size: "サイズ",
  confidence: "確度",
};

// --- 表示ヘルパ -------------------------------------------------------------

/** 入手元の経路を日本語にする。英語のままだと何のことか分からない。 */
const SOURCE_LABEL = {
  browser_ext: "ブラウザ拡張",
  zone_identifier: "Zone.Identifier",
  xattr: "拡張属性",
  gvfs: "GVFS",
  manual: "手動登録",
  history_db: "ブラウザ履歴",
};

const CONFIDENCE_LABEL = {
  certain: "確定", high: "高", medium: "中", low: "低",
};

function fmtSize(n) {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
}

function fmtTime(sec) {
  if (!sec || sec <= 0) return "";
  const d = new Date(sec * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** textContent 経由でのみ文字列を入れる。innerHTML は使わない。 */
function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

let toastTimer = null;
function toast(msg) {
  els.toast.textContent = msg;
  els.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { els.toast.hidden = true; }, 3200);
}

// --- 検索 -------------------------------------------------------------------

let searchTimer = null;
function scheduleSearch() {
  clearTimeout(searchTimer);
  // 打鍵ごとに DB を叩かない。体感で遅れない範囲。
  searchTimer = setTimeout(runSearch, 180);
}

async function runSearch() {
  try {
    const hits = await invoke("search", {
      name: els.q.value || null,
      host: els.qhost.value || null,
      url: null,
      limit: 300,
      sort: sort.key,
      descending: sort.descending,
    });
    renderList(hits);
  } catch (e) {
    els.count.textContent = `検索できません: ${e}`;
  }
}

function renderList(hits) {
  els.results.replaceChildren();
  els.count.textContent = hits.length
    ? `${hits.length} 件`
    : "該当なし — 取り込みがまだなら「取り込み」から始めてください";

  for (const h of hits) {
    const li = el("li");
    li.tabIndex = 0;
    li.dataset.id = String(h.id);
    li.setAttribute("aria-selected", String(h.id === selectedId));

    li.appendChild(el("div", "name", h.name));

    if (h.url) {
      li.appendChild(el("div", "origin", h.url));
    } else {
      li.appendChild(el("div", "origin none", "入手元の記録なし"));
    }

    const meta = el("div", "meta");
    if (h.confidence) {
      meta.appendChild(el("span", `tag ${h.confidence}`, CONFIDENCE_LABEL[h.confidence] || h.confidence));
    }
    if (h.status === "missing") {
      meta.appendChild(el("span", "tag missing", "見失い中"));
    }
    if (h.source) meta.appendChild(el("span", null, SOURCE_LABEL[h.source] || h.source));
    meta.appendChild(el("span", null, fmtSize(h.size)));
    const when = fmtTime(h.acquired_at);
    if (when) meta.appendChild(el("span", null, when));
    li.appendChild(meta);

    li.addEventListener("click", () => select(h.id));
    li.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(h.id); }
    });
    els.results.appendChild(li);
  }
}

// --- 詳細 -------------------------------------------------------------------

async function select(id) {
  selectedId = id;
  for (const li of els.results.children) {
    li.setAttribute("aria-selected", String(Number(li.dataset.id) === id));
  }
  document.body.classList.add("showing-detail");

  try {
    renderDetail(await invoke("detail", { id }));
  } catch (e) {
    els.detail.replaceChildren(el("div", "empty", `詳細を読めません: ${e}`));
  }
}

function renderDetail(d) {
  const root = document.createDocumentFragment();

  const name = d.path.split(/[\\/]/).pop() || d.path;
  root.appendChild(el("h2", null, name));
  root.appendChild(el("div", "path", d.path));

  // 入手元 — この画面の主役なので最初に出す。
  root.appendChild(el("h3", null, "入手元"));
  if (!d.origins.length) {
    root.appendChild(el("div", "card sub", "記録がありません。「取り込み」や手動登録で追加できます。"));
  }
  for (const o of d.origins) {
    const card = el("div", "card");
    card.appendChild(el("div", "url", o.url || "(URL なし)"));
    if (o.referrer_url) card.appendChild(el("div", "sub", `参照元: ${o.referrer_url}`));

    const meta = el("div", "meta");
    meta.appendChild(el("span", `tag ${o.confidence}`, CONFIDENCE_LABEL[o.confidence] || o.confidence));
    meta.appendChild(el("span", null, SOURCE_LABEL[o.source] || o.source));
    if (o.browser) meta.appendChild(el("span", null, o.browser));
    const when = fmtTime(o.acquired_at);
    if (when) meta.appendChild(el("span", null, when));
    if (o.inherited) meta.appendChild(el("span", "tag inherited", "コピー元から継承"));
    card.appendChild(meta);
    root.appendChild(card);
  }

  if (d.lineage.length) {
    root.appendChild(el("h3", null, "コピー元"));
    const card = el("div", "card");
    for (const p of d.lineage) card.appendChild(el("div", "sub", p));
    root.appendChild(card);
  }

  // パス履歴 — 1 件しか無ければ「動いていない」ので出さない。
  if (d.paths.length > 1) {
    root.appendChild(el("h3", null, "パス履歴"));
    const ol = el("ol", "paths");
    for (const p of d.paths) {
      const li = el("li");
      li.appendChild(el("span", "when", `${fmtTime(p.observed_at)}${p.is_current ? "  現在" : ""}`));
      li.appendChild(el("div", null, p.path));
      ol.appendChild(li);
    }
    root.appendChild(ol);
  }

  root.appendChild(el("h3", null, "ファイル"));
  const dl = el("dl", "facts");
  const fact = (k, v) => { dl.appendChild(el("dt", null, k)); dl.appendChild(el("dd", null, v)); };
  fact("サイズ", fmtSize(d.size));
  fact("SHA-256", d.sha256 || "(未計算)");
  fact("識別子", d.stable_id);
  fact("更新日時", fmtTime(d.mtime) || "(不明)");
  fact("状態", { present: "あり", missing: "見失い中", deleted: "削除済み" }[d.status] || d.status);
  root.appendChild(dl);

  els.detail.replaceChildren(root);
}

// --- 取り込み・状態 ---------------------------------------------------------

$("btn-scan").addEventListener("click", async () => {
  // 既定のダウンロードフォルダを初期値に入れておく。
  try {
    const s = await invoke("status");
    if (s.download_dirs.length && !$("scan-path").value) {
      $("scan-path").value = s.download_dirs[0];
    }
  } catch (_) { /* 状態が取れなくても取り込みは開ける */ }
  $("dlg-scan").showModal();
});

$("dlg-scan").addEventListener("close", async (e) => {
  if ($("dlg-scan").returnValue !== "go") return;
  const path = $("scan-path").value.trim();
  if (!path) { toast("パスを入力してください"); return; }

  toast("取り込み中…");
  try {
    toast(await invoke("scan", { path, hash: $("scan-hash").checked }));
    // 並べ替えの選択肢を先に用意してから検索する。
// 先に検索すると、保存済みの並びが反映されないまま一瞬既定で描画される。
setupSort().then(runSearch);
  } catch (err) {
    toast(`取り込めません: ${err}`);
  }
});

$("btn-status").addEventListener("click", async () => {
  const body = $("status-body");
  body.replaceChildren(el("div", "sub", "読み込み中…"));
  $("dlg-status").showModal();
  try {
    const s = await invoke("status");
    const dl = el("dl", "facts");
    const fact = (k, v) => { dl.appendChild(el("dt", null, k)); dl.appendChild(el("dd", null, v)); };
    fact("記録数", `${s.files} ファイル`);
    fact("プラットフォーム", s.platform);
    fact("デーモン", s.daemon_running ? "稼働中" : "停止中（自動記録は無効）");
    fact("IPC", s.ipc_endpoint);
    fact("DB", s.db_path);
    if (s.download_dirs.length) fact("ダウンロード", s.download_dirs.join("\n"));

    body.replaceChildren(dl);
    if (!s.daemon_running) {
      body.appendChild(el("div", "card sub",
        "fo-daemon を起動すると、ダウンロードの自動記録とファイル移動の追従が有効になります。"));
    }
    for (const a of s.advice) body.appendChild(el("div", "card sub", a));
  } catch (e) {
    body.replaceChildren(el("div", "card sub", `状態を取れません: ${e}`));
  }
});

// --- 起動 -------------------------------------------------------------------

// --- 並べ替え ---------------------------------------------------------------

/** 向きのボタンの文字。何順なのかが一目で分かる言葉にする。 */
function dirLabel() {
  if (sort.key === "name") return sort.descending ? "Z → A" : "A → Z";
  if (sort.key === "size") return sort.descending ? "大 → 小" : "小 → 大";
  if (sort.key === "confidence") return sort.descending ? "高 → 低" : "低 → 高";
  return sort.descending ? "新 → 旧" : "旧 → 新";
}

function renderSortUi() {
  els.sortKey.value = sort.key;
  els.sortDir.textContent = dirLabel();
}

function saveSort() {
  // 記録できなくても検索は動く。失敗させない。
  try {
    localStorage.setItem("fo.sort", JSON.stringify(sort));
  } catch (_) { /* プライベートウィンドウ相当の環境でも動くように */ }
}

function loadSort(options) {
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem("fo.sort") || "null");
  } catch (_) { /* 壊れていたら既定に戻す */ }

  const known = options.find((o) => o.key === (saved && saved.key));
  if (known) {
    sort = {
      key: known.key,
      descending:
        typeof saved.descending === "boolean" ? saved.descending : known.default_descending,
    };
  }
}

async function setupSort() {
  let options;
  try {
    options = await invoke("sort_options");
  } catch (e) {
    // 並べ替えが使えなくても一覧は出す。
    els.sortKey.parentElement.hidden = true;
    els.sortDir.hidden = true;
    return;
  }

  for (const o of options) {
    const opt = document.createElement("option");
    opt.value = o.key;
    opt.textContent = SORT_LABEL[o.key] || o.key;
    els.sortKey.appendChild(opt);
  }

  loadSort(options);
  renderSortUi();

  els.sortKey.addEventListener("change", () => {
    const chosen = options.find((o) => o.key === els.sortKey.value);
    // 項目を変えたら、その項目の自然な向きに戻す。
    // 名前順に切り替えたときに Z から始まると使いにくい。
    sort = { key: chosen.key, descending: chosen.default_descending };
    renderSortUi();
    saveSort();
    runSearch();
  });

  els.sortDir.addEventListener("click", () => {
    sort.descending = !sort.descending;
    renderSortUi();
    saveSort();
    runSearch();
  });
}

els.q.addEventListener("input", scheduleSearch);
els.qhost.addEventListener("input", scheduleSearch);

// 狭い画面で詳細から一覧へ戻る。
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && document.body.classList.contains("showing-detail")) {
    document.body.classList.remove("showing-detail");
  }
});

runSearch();
