// File Origin GUI のフロントエンド。
//
// バンドラを使わない素の JS。理由は 2 つ:
//  - この画面の規模（検索・一覧・詳細）ならフレームワークの利得が無い
//  - npm を挟まない分、`cargo tauri build` だけでビルドが完結する
// 規模が増えたら D5 を見直す。
//
// 見た目はデザインシステムの部品（fo- で始まるクラス）に載せている。
// ここで新しいクラスを足さない。足したくなったら assets/design-system/ 側に部品を作る。
// 決まりごとは docs/design-system.md。

const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);
const els = {
  q: $("q"), chips: $("chips"), query: $("query"),
  results: $("results"), count: $("count"), detail: $("detail"),
  toasts: $("toasts"), statusbar: $("statusbar"),
  sortKey: $("sort-key"), sortDir: $("sort-dir"),
};

let selectedId = null;

/** 最後に取れた状態。ステータスバーと空状態の文言に使う。 */
let statusCache = null;

/** 並び順。選択は localStorage に覚えさせる。 */
let sort = { key: "first-seen", descending: true };

/** 検索の絞り込み。QueryBar のチップになる。 */
const filters = { host: null, url: null };

/** チップにできる絞り込み。Rust 側の `search` が受け取れるものだけ。 */
const FILTER_KEYS = ["host", "url"];

/** 一度に描く上限。これに届いたら件数の表示で「打ち切った」と分かるようにする。 */
const LIMIT = 300;

/** 並べ替えの項目の表示名。キーの一覧は Rust 側（sort_options）が持つ。 */
const SORT_LABEL = {
  "first-seen": "取り込み順",
  acquired: "取得日時",
  name: "ファイル名",
  size: "サイズ",
  confidence: "確度",
};

/** 入手元の経路を日本語にする。英語のままだと何のことか分からない。 */
const SOURCE_LABEL = {
  browser_ext: "ブラウザ拡張",
  zone_identifier: "Zone.Identifier",
  xattr: "拡張属性",
  gvfs: "GVFS",
  manual: "手動登録",
  history_db: "ブラウザ履歴",
};

/** 経路ごとのアイコン。どこから来た情報かを形で見分けられるように。 */
const SOURCE_ICON = {
  browser_ext: "plug",
  zone_identifier: "file",
  xattr: "file",
  gvfs: "file",
  manual: "pencil",
  history_db: "database",
};

const CONFIDENCE_LABEL = { certain: "確定", high: "高", medium: "中", low: "低" };

/** 状態は ○ △ × と語の両方で出す。色だけで区別させない。 */
const STATUS_MARK = {
  present: { mark: "○", text: "あり", tone: "ok" },
  missing: { mark: "△", text: "見失い中", tone: "warn" },
  deleted: { mark: "×", text: "削除済み", tone: "danger" },
};

// --- DOM の組み立て ---------------------------------------------------------

/** textContent 経由でのみ文字列を入れる。innerHTML は使わない。 */
function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

/** アイコン。CSS マスクなので色は currentColor に従う。 */
function icon(name, size) {
  const n = el("span", `fo-i fo-i-${name}${size ? ` fo-i-${size}` : ""}`);
  n.setAttribute("aria-hidden", "true");
  return n;
}

function btn(cls, label, onClick, title) {
  const b = el("button", cls ? `fo-btn ${cls}` : "fo-btn");
  b.type = "button";
  if (label) b.append(label);
  if (title) { b.title = title; b.setAttribute("aria-label", title); }
  if (onClick) b.addEventListener("click", onClick);
  return b;
}

/** アイコンだけのボタン。名前は title と aria-label で補う。 */
function iconBtn(name, title, onClick, cls) {
  const b = btn(`${cls || "fo-btn-ghost"} fo-btn-sm fo-btn-icon`, null, onClick, title);
  b.appendChild(icon(name, "sm"));
  return b;
}

// --- 値の見せ方 -------------------------------------------------------------

function fmtSize(n) {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
}

const pad2 = (n) => String(n).padStart(2, "0");

function fmtTime(sec) {
  if (!sec || sec <= 0) return "";
  const d = new Date(sec * 1000);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ` +
    `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** 一覧行の日時。年は要らない（並びで分かる）ので月日と時刻だけ。 */
function fmtShortTime(sec) {
  if (!sec || sec <= 0) return "";
  const d = new Date(sec * 1000);
  return `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function fmtCount(n) {
  return Number(n).toLocaleString("ja-JP");
}

const SEPS = ["/", "\\"];

function baseName(p) {
  let cut = -1;
  for (const s of SEPS) cut = Math.max(cut, p.lastIndexOf(s));
  return cut >= 0 ? p.slice(cut + 1) : p;
}

function dirName(p) {
  let cut = -1;
  for (const s of SEPS) cut = Math.max(cut, p.lastIndexOf(s));
  return cut >= 0 ? p.slice(0, cut) : "";
}

/**
 * URL をホストとそれ以外に分ける。ホストが誰なのかが一番大事な情報なので、
 * そこだけ色を付けて、長い経路は沈める。
 *
 * `withScheme` は詳細（入手元カード）だけ true。一覧では場所を食うので落とす。
 */
function urlNode(raw, { withScheme = false, quiet = false } = {}) {
  const span = el("span", quiet ? "fo-url fo-quiet" : "fo-url");
  let u = null;
  try { u = new URL(raw); } catch (_) { /* 解釈できなければそのまま出す */ }

  if (!u || !u.host) {
    // file:// や解釈できないものは、丸ごと経路として出す。
    span.appendChild(el("span", "fo-path", raw));
    return span;
  }
  if (withScheme) span.appendChild(el("span", "fo-scheme", `${u.protocol}//`));
  span.appendChild(el("span", "fo-host", u.host));
  const rest = `${u.pathname}${u.search}${u.hash}`;
  if (rest && rest !== "/") span.appendChild(el("span", "fo-path", rest));
  return span;
}

/** 確度メーター。4 本のバーの点灯数で高さを示す。 */
function confidenceNode(level, { withLabel = false } = {}) {
  const lv = level || "none";
  const n = el("span", "fo-conf");
  n.dataset.level = lv;
  const label = CONFIDENCE_LABEL[lv] || "—";

  const bars = el("span", "fo-conf-bars");
  for (let i = 0; i < 4; i++) bars.appendChild(el("i"));
  n.appendChild(bars);

  if (withLabel) {
    n.appendChild(el("span", "fo-conf-label", label));
  } else {
    // 目で見えるのはバーだけなので、読み上げ用に語を添える。
    n.title = `確度: ${label}`;
    n.appendChild(el("span", "fo-sr", `確度 ${label}`));
  }
  return n;
}

/** 入手元の経路のタグ。 */
function sourceNode(source) {
  const tag = el("span", "fo-src");
  tag.appendChild(icon(SOURCE_ICON[source] || "info"));
  const label = SOURCE_LABEL[source] || source;
  // 機械が付けた名前はそのまま等幅で出す。訳すと元が分からなくなる。
  tag.appendChild(source === "zone_identifier" ? el("code", null, label) : el("span", null, label));
  return tag;
}

/** ○ △ × と語。記号だけでも語だけでも意味が通るように両方出す。 */
function markNode(mark, text, tone) {
  const n = el("span", "fo-mark");
  n.dataset.tone = tone;
  n.appendChild(el("b", null, mark));
  n.append(text);
  return n;
}

function statusMarkNode(status) {
  const s = STATUS_MARK[status] || { mark: "?", text: status, tone: "neutral" };
  return markNode(s.mark, s.text, s.tone);
}

/** SHA-256 は 8 桁ずつに割る。64 桁の一本棒は目で追えない。 */
function hashNode(hex) {
  const n = el("span", "fo-hash");
  for (let i = 0; i < hex.length; i += 8) n.appendChild(el("span", null, hex.slice(i, i + 8)));
  return n;
}

// --- トースト ---------------------------------------------------------------

const TOAST_ICON = { ok: "check", danger: "alert" };

function toast(msg, tone) {
  const t = el("div", "fo-toast");
  if (tone) t.dataset.tone = tone;
  t.appendChild(icon(TOAST_ICON[tone] || "info"));
  t.append(msg);
  els.toasts.appendChild(t);
  setTimeout(() => t.remove(), 3600);
}

/**
 * クリップボードへ。
 *
 * Tauri の webview は secure context にならないことがあり、
 * その環境では navigator.clipboard が無い。古い経路に落とす。
 */
async function copyText(text, what) {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      toast(`${what}をコピーしました`, "ok");
      return;
    }
  } catch (_) { /* 落ちたら下の経路を試す */ }

  try {
    const ta = el("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    toast(ok ? `${what}をコピーしました` : `${what}をコピーできません`, ok ? "ok" : "danger");
  } catch (_) {
    toast(`${what}をコピーできません`, "danger");
  }
}

// --- QueryBar ---------------------------------------------------------------

/**
 * 入力欄から `host:` `url:` を切り出してチップにする。
 *
 * 語を打ち終えた合図（末尾の空白）を見てから動かす。打鍵の途中で
 * 入力欄の中身が消えると、何が起きたのか分からなくなる。
 */
function harvestFilters() {
  const v = els.q.value;
  if (!v.endsWith(" ")) return false;

  const keep = [];
  let changed = false;
  for (const tok of v.split(" ")) {
    if (!tok) continue;
    const i = tok.indexOf(":");
    const key = i > 0 ? tok.slice(0, i).toLowerCase() : "";
    if (FILTER_KEYS.includes(key) && tok.length > i + 1) {
      filters[key] = tok.slice(i + 1);
      changed = true;
    } else {
      keep.push(tok);
    }
  }
  if (!changed) return false;

  els.q.value = keep.length ? `${keep.join(" ")} ` : "";
  renderChips();
  return true;
}

function renderChips() {
  els.chips.replaceChildren();
  for (const key of FILTER_KEYS) {
    const value = filters[key];
    if (!value) continue;
    const chip = el("span", "fo-chip");
    chip.appendChild(el("b", null, `${key}:`));
    chip.append(value);

    // 外すボタン。見た目は fo-chip 側が持っているので fo-btn は着せない。
    const x = el("button");
    x.type = "button";
    x.title = `${key}: の絞り込みを外す`;
    x.setAttribute("aria-label", x.title);
    x.appendChild(icon("close"));
    x.addEventListener("click", () => {
      filters[key] = null;
      renderChips();
      runSearch();
    });
    chip.appendChild(x);
    els.chips.appendChild(chip);
  }
}

function anyFilter() {
  return Boolean(els.q.value.trim() || filters.host || filters.url);
}

function clearFilters() {
  els.q.value = "";
  for (const k of FILTER_KEYS) filters[k] = null;
  renderChips();
  runSearch();
}

// --- 検索 -------------------------------------------------------------------

let searchTimer = null;
function scheduleSearch() {
  if (harvestFilters()) { runSearch(); return; }
  clearTimeout(searchTimer);
  // 打鍵ごとに DB を叩かない。体感で遅れない範囲。
  searchTimer = setTimeout(runSearch, 180);
}

async function runSearch() {
  try {
    const hits = await invoke("search", {
      name: els.q.value.trim() || null,
      host: filters.host,
      url: filters.url,
      limit: LIMIT,
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

  // 上限に届いたら「これで全部」と読ませない。絞り込みを促す。
  const capped = hits.length >= LIMIT;
  els.count.replaceChildren(el("b", null, fmtCount(hits.length)),
    document.createTextNode(capped ? " 件以上（表示はここまで）" : " 件"));
  els.count.title = capped ? "絞り込むと残りが見えます" : "";

  if (!hits.length) {
    els.results.appendChild(emptyListNode());
    return;
  }

  for (const h of hits) {
    els.results.appendChild(rowNode(h));
  }
}

function rowNode(h) {
  const li = el("li", h.status === "missing" ? "fo-row is-missing" : "fo-row");
  li.tabIndex = 0;
  li.dataset.id = String(h.id);
  li.setAttribute("aria-selected", String(h.id === selectedId));

  li.appendChild(confidenceNode(h.confidence));

  const main = el("div", "fo-row-main");
  const name = el("div", "fo-row-name");
  name.appendChild(el("span", null, h.name));
  if (h.status === "missing") name.appendChild(statusMarkNode(h.status));
  main.appendChild(name);

  if (h.url) {
    const origin = el("div", "fo-row-origin");
    origin.appendChild(urlNode(h.url, { quiet: true }));
    origin.title = h.url;
    main.appendChild(origin);
  } else {
    main.appendChild(el("div", "fo-row-none", "入手元の記録なし"));
  }
  li.appendChild(main);

  const side = el("div", "fo-row-side");
  const when = fmtShortTime(h.acquired_at);
  if (when) side.appendChild(el("span", null, when));
  side.appendChild(el("span", null, fmtSize(h.size)));
  li.appendChild(side);

  li.addEventListener("click", () => select(h.id));
  li.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(h.id); }
  });
  return li;
}

/** 一覧が空のとき。まだ何も無いのか、絞り込みが強すぎるのかで言うことが違う。 */
function emptyListNode() {
  const slot = el("li", "fo-empty-slot");
  const box = el("div", "fo-empty");
  const nothingYet = statusCache && statusCache.files === 0;

  box.appendChild(icon(nothingYet ? "import" : "search", "lg"));
  if (nothingYet) {
    box.appendChild(el("h2", null, "まだ何も記録されていません"));
    box.appendChild(el("p", null,
      "「取り込み」からダウンロードフォルダを読むと、入手元の記録が集まります。"));
    const acts = el("div", "fo-empty-actions");
    acts.appendChild(btn("fo-btn-primary", "取り込み", openScan));
    box.appendChild(acts);
  } else {
    box.appendChild(el("h2", null, "該当なし"));
    box.appendChild(el("p", null, "名前は * と ? が使えます。host: と url: でも絞り込めます。"));
    if (anyFilter()) {
      const acts = el("div", "fo-empty-actions");
      acts.appendChild(btn("", "絞り込みを外す", clearFilters));
      box.appendChild(acts);
    }
  }
  slot.appendChild(box);
  return slot;
}

// --- 詳細 -------------------------------------------------------------------

async function select(id) {
  selectedId = id;
  for (const li of els.results.children) {
    if (li.classList.contains("fo-row")) {
      li.setAttribute("aria-selected", String(Number(li.dataset.id) === id));
    }
  }
  document.body.classList.add("showing-detail");

  try {
    renderDetail(await invoke("detail", { id }));
  } catch (e) {
    els.detail.replaceChildren(noticeNode("danger", "詳細を読めません", String(e)));
  }
}

function section(label, count) {
  const sec = el("section", "fo-section");
  const head = el("div", "fo-section-head");
  head.appendChild(el("h3", "fo-label", label));
  if (count) head.appendChild(el("span", "fo-count", count));
  sec.appendChild(head);
  return sec;
}

function noticeNode(tone, title, body) {
  const box = el("div", "fo-callout");
  if (tone) box.dataset.tone = tone;
  box.appendChild(icon(tone === "danger" ? "alert" : "info"));
  const inner = el("div");
  inner.appendChild(el("p", "fo-callout-title", title));
  if (body) inner.appendChild(el("p", null, body));
  box.appendChild(inner);
  return box;
}

function renderDetail(d) {
  const root = document.createDocumentFragment();
  const wrap = el("div", "fo-detail");
  const best = d.origins.length ? d.origins[0] : null;

  // 見出し ---------------------------------------------------------------
  const head = el("header", "fo-detail-head");
  head.appendChild(el("h2", "fo-detail-title", baseName(d.path)));
  head.appendChild(el("div", "fo-detail-path", d.path));

  const acts = el("div", "fo-detail-actions");
  const back = btn("fo-btn-sm fo-detail-back", "一覧へ戻る",
    () => document.body.classList.remove("showing-detail"));
  acts.appendChild(back);
  if (best && best.url) {
    const b = btn("fo-btn-sm", "URL をコピー", () => copyText(best.url, "URL"));
    b.prepend(icon("copy", "sm"));
    acts.appendChild(b);
  }
  const bp = btn("fo-btn-sm", "パスをコピー", () => copyText(d.path, "パス"));
  bp.prepend(icon("copy", "sm"));
  acts.appendChild(bp);
  head.appendChild(acts);
  wrap.appendChild(head);

  // 入手元 — この画面の主役なので最初に出す。
  const conf = d.origins.length > 1 ? `${d.origins.length} 件 · 確度の高い順` : null;
  const secOrigins = section("入手元", conf);
  const list = el("div", "fo-origins");
  if (!d.origins.length) {
    const none = el("article", "fo-origin is-empty");
    none.append("記録がありません。取り込みや手動登録（fo add）で追加できます。");
    list.appendChild(none);
  }
  for (const o of d.origins) list.appendChild(originNode(o));
  secOrigins.appendChild(list);
  wrap.appendChild(secOrigins);

  // 来歴 — パスが 1 件でも出す。取得のノードがあるので意味がある。
  if (d.paths.length) wrap.appendChild(lineageSection(d, best));

  // コピー元 — 祖先の現在地。時刻は持っていないので時刻の桁は畳む。
  if (d.lineage.length) {
    const sec = section("コピー元", `${d.lineage.length} 件`);
    const ol = el("ol", "fo-tl is-timeless");
    for (const p of d.lineage) {
      const li = el("li", "fo-tl-item");
      li.dataset.kind = "copy";
      li.appendChild(el("span", "fo-tl-time"));
      const node = el("span", "fo-tl-node");
      node.setAttribute("aria-hidden", "true");
      li.appendChild(node);
      const body = el("div", "fo-tl-body");
      body.appendChild(el("div", "fo-tl-path", p));
      li.appendChild(body);
      ol.appendChild(li);
    }
    sec.appendChild(ol);
    wrap.appendChild(sec);
  }

  // ファイル -------------------------------------------------------------
  const secFile = section("ファイル");
  const dl = el("dl", "fo-facts");
  const fact = (k, v) => {
    dl.appendChild(el("dt", null, k));
    const dd = el("dd");
    if (v instanceof Node) dd.appendChild(v); else dd.append(v);
    dl.appendChild(dd);
    return dd;
  };

  const size = el("span");
  size.append(fmtSize(d.size), " ");
  size.appendChild(el("span", "fo-muted", `(${fmtCount(d.size)} B)`));
  fact("サイズ", size);

  if (d.sha256) {
    const dd = fact("SHA-256", hashNode(d.sha256));
    dd.appendChild(iconBtn("copy", "SHA-256 をコピー", () => copyText(d.sha256, "SHA-256")));
  } else {
    fact("SHA-256", el("span", "fo-muted", "未計算"));
  }

  fact("識別子", d.stable_id);
  fact("更新日時", fmtTime(d.mtime) || "不明");
  fact("状態", statusMarkNode(d.status));
  secFile.appendChild(dl);
  wrap.appendChild(secFile);

  // CLI ------------------------------------------------------------------
  const secCli = section("CLI");
  const cmd = `fo show "${d.path}"`;
  const line = el("div", "fo-cmd");
  line.appendChild(el("code", null, cmd));
  line.appendChild(iconBtn("copy", "コマンドをコピー", () => copyText(cmd, "コマンド")));
  secCli.appendChild(line);
  wrap.appendChild(secCli);

  root.appendChild(wrap);
  els.detail.replaceChildren(root);
  els.detail.scrollTop = 0;
}

function originNode(o) {
  const card = el("article", "fo-origin");

  const head = el("header", "fo-origin-head");
  head.appendChild(confidenceNode(o.confidence, { withLabel: true }));
  head.appendChild(sourceNode(o.source));
  const meta = [o.browser, fmtTime(o.acquired_at)].filter(Boolean).join(" · ");
  if (meta) head.appendChild(el("span", "fo-origin-meta", meta));
  if (o.inherited) {
    const badge = el("span", "fo-badge", "継承");
    badge.dataset.tone = "accent";
    badge.title = "コピー元から引き継いだ記録";
    head.appendChild(badge);
  }
  if (o.url) {
    const a = el("span", "fo-origin-actions");
    a.appendChild(iconBtn("copy", "URL をコピー", () => copyText(o.url, "URL")));
    head.appendChild(a);
  }
  card.appendChild(head);

  const url = el("div", "fo-origin-url");
  if (o.url) url.appendChild(urlNode(o.url, { withScheme: true }));
  else url.appendChild(el("span", "fo-muted", "URL の記録なし"));
  card.appendChild(url);

  if (o.referrer_url) {
    const ref = el("dl", "fo-origin-ref");
    ref.appendChild(el("dt", null, "参照元"));
    ref.appendChild(el("dd", null, o.referrer_url));
    card.appendChild(ref);
  }
  return card;
}

/** 移動の種類。パスのどこが変わったかで呼び分ける。 */
function moveKind(prev, cur) {
  const dirChanged = dirName(prev) !== dirName(cur);
  const nameChanged = baseName(prev) !== baseName(cur);
  if (dirChanged && nameChanged) return ["rename", "移動・名前変更"];
  if (dirChanged) return ["move", "移動"];
  if (nameChanged) return ["rename", "名前変更"];
  return ["move", "再確認"];
}

/** 前のパスと共通する頭を探し、変わった部分だけ強調する。 */
function pathSplit(prev, cur) {
  if (!prev) return ["", cur];
  let cut = 0;
  const n = Math.min(prev.length, cur.length);
  for (let i = 0; i < n; i++) {
    if (prev[i] !== cur[i]) break;
    if (cur[i] === "/" || cur[i] === "\\") cut = i + 1;
  }
  return [cur.slice(0, cut), cur.slice(cut)];
}

function lineageSection(d, best) {
  const moves = d.paths.length - 1;
  const sec = section("来歴", moves > 0 ? `パス ${d.paths.length} 件` : null);
  const ol = el("ol", "fo-tl");

  d.paths.forEach((p, i) => {
    const li = el("li", "fo-tl-item");
    const first = i === 0;
    const prev = first ? null : d.paths[i - 1].path;

    // 1 件目は「手に入れた瞬間」。取得日時があればそちらを使う。
    let kind = "acquire";
    let title = best && best.acquired_at ? "取得" : "初めて記録";
    let at = first && best && best.acquired_at ? best.acquired_at : p.observed_at;
    if (!first) {
      [kind, title] = moveKind(prev, p.path);
      at = p.observed_at;
    }
    li.dataset.kind = kind;
    if (p.is_current) li.classList.add("is-current");

    const time = el("span", "fo-tl-time");
    const stamp = fmtTime(at);
    if (stamp) {
      const [day, clock] = stamp.split(" ");
      time.appendChild(el("span", null, day));
      time.appendChild(el("span", null, clock));
    }
    li.appendChild(time);

    const node = el("span", "fo-tl-node");
    node.setAttribute("aria-hidden", "true");
    li.appendChild(node);

    const body = el("div", "fo-tl-body");
    const head = el("div", "fo-tl-title", title);
    if (first && best) {
      head.appendChild(el("span", "fo-muted",
        [SOURCE_LABEL[best.source] || best.source, best.browser].filter(Boolean).join(" · ")));
    }
    if (p.is_current) {
      const badge = el("span", "fo-badge", "現在");
      badge.dataset.tone = "ok";
      head.appendChild(badge);
    }
    body.appendChild(head);

    const path = el("div", "fo-tl-path");
    const [keep, tail] = pathSplit(prev, p.path);
    if (keep) path.append(keep);
    path.appendChild(keep ? el("b", null, tail) : document.createTextNode(tail));
    body.appendChild(path);

    li.appendChild(body);
    ol.appendChild(li);
  });

  sec.appendChild(ol);
  return sec;
}

// --- ステータスバー ---------------------------------------------------------

function renderStatusBar(s) {
  els.statusbar.replaceChildren();

  const daemon = el("button", "fo-sb-item");
  daemon.type = "button";
  const dot = el("span", "fo-dot");
  dot.dataset.tone = s.daemon_running ? "ok" : "danger";
  daemon.appendChild(dot);
  daemon.append(s.daemon_running ? "デーモン稼働中" : "デーモン停止中");
  daemon.title = s.ipc_endpoint;
  daemon.addEventListener("click", openStatus);
  els.statusbar.appendChild(daemon);

  if (s.advice.length) {
    const warn = el("button", "fo-sb-item");
    warn.type = "button";
    warn.dataset.tone = "danger";
    warn.appendChild(icon("alert"));
    warn.append(`注意 ${s.advice.length} 件`);
    warn.addEventListener("click", openStatus);
    els.statusbar.appendChild(warn);
  }

  els.statusbar.appendChild(el("span", "fo-sb-spacer"));
  els.statusbar.appendChild(el("span", "fo-sb-item fo-mono", `記録 ${fmtCount(s.files)} 件`));
  els.statusbar.appendChild(el("span", "fo-sb-item fo-mono fo-sb-optional", s.platform));
}

async function refreshStatus() {
  try {
    statusCache = await invoke("status");
    renderStatusBar(statusCache);
  } catch (e) {
    els.statusbar.replaceChildren(el("span", "fo-sb-item", `状態を取れません: ${e}`));
  }
  return statusCache;
}

// --- 取り込み・状態 ---------------------------------------------------------

async function openScan() {
  // 既定のダウンロードフォルダを初期値に入れておく。
  const s = statusCache || (await refreshStatus());
  if (s && s.download_dirs.length && !$("scan-path").value) {
    $("scan-path").value = s.download_dirs[0];
  }
  $("dlg-scan").showModal();
  $("scan-path").focus();
}

async function openStatus() {
  const body = $("status-body");
  body.replaceChildren(el("div", "fo-muted", "読み込み中…"));
  $("dlg-status").showModal();

  const s = await refreshStatus();
  if (!s) {
    body.replaceChildren(noticeNode("danger", "状態を取れません"));
    return;
  }

  const dl = el("dl", "fo-facts");
  const fact = (k, v) => {
    dl.appendChild(el("dt", null, k));
    const dd = el("dd");
    if (v instanceof Node) dd.appendChild(v); else dd.append(v);
    dl.appendChild(dd);
  };
  fact("記録数", `${fmtCount(s.files)} ファイル`);
  fact("プラットフォーム", s.platform);
  fact("デーモン", s.daemon_running
    ? markNode("○", "稼働中", "ok")
    : markNode("×", "停止中", "danger"));
  fact("IPC", s.ipc_endpoint);
  fact("DB", s.db_path);
  if (s.download_dirs.length) {
    const dirs = el("span");
    for (const p of s.download_dirs) dirs.appendChild(el("div", null, p));
    fact("ダウンロード", dirs);
  }

  body.replaceChildren(dl);
  if (!s.daemon_running) {
    body.appendChild(noticeNode(null, "自動記録は止まっています",
      "fo-daemon を起動すると、ダウンロードの記録とファイル移動の追従が有効になります。"));
  }
  for (const a of s.advice) body.appendChild(noticeNode("warn", a));
}

$("btn-scan").addEventListener("click", openScan);
$("btn-status").addEventListener("click", openStatus);

// ダイアログの閉じるボタン。value は close() にそのまま渡す。
for (const b of document.querySelectorAll("dialog [data-close]")) {
  b.addEventListener("click", () => b.closest("dialog").close(b.dataset.close));
}
$("scan-go").addEventListener("click", () => $("dlg-scan").close("go"));

$("dlg-scan").addEventListener("close", async () => {
  if ($("dlg-scan").returnValue !== "go") return;
  const path = $("scan-path").value.trim();
  if (!path) { toast("パスを入力してください", "danger"); return; }

  toast("取り込み中…");
  try {
    toast(await invoke("scan", { path, hash: $("scan-hash").checked }), "ok");
    await refreshStatus();
    runSearch();
  } catch (err) {
    toast(`取り込めません: ${err}`, "danger");
  }
});

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
  } catch (_) {
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

// --- 起動 -------------------------------------------------------------------

els.q.addEventListener("input", scheduleSearch);
els.q.addEventListener("blur", () => {
  // 末尾に空白が無くても、離れたときには確定させる。
  if (els.q.value && !els.q.value.endsWith(" ")) {
    els.q.value += " ";
    if (harvestFilters()) runSearch(); else els.q.value = els.q.value.slice(0, -1);
  }
});
els.q.addEventListener("keydown", (e) => {
  // 空の入力欄で Backspace を押したら、最後のチップを外す。
  if (e.key !== "Backspace" || els.q.value) return;
  for (const k of [...FILTER_KEYS].reverse()) {
    if (filters[k]) { filters[k] = null; renderChips(); runSearch(); return; }
  }
});

function dialogOpen() {
  return Boolean(document.querySelector("dialog[open]"));
}

window.addEventListener("keydown", (e) => {
  if (dialogOpen()) return;

  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
    e.preventDefault();
    els.q.focus();
    els.q.select();
    return;
  }
  // 狭い画面で詳細から一覧へ戻る。
  if (e.key === "Escape" && document.body.classList.contains("showing-detail")) {
    document.body.classList.remove("showing-detail");
  }
});

function detailEmptyNode() {
  const box = el("div", "fo-empty");
  box.appendChild(icon("file", "lg"));
  box.appendChild(el("h2", null, "ファイルを選んでください"));
  box.appendChild(el("p", null, "左の一覧から選ぶと、入手元と来歴が出ます。"));
  return box;
}

els.detail.replaceChildren(detailEmptyNode());

// 並べ替えの選択肢と記録数を先に揃えてから検索する。
// 先に検索すると、保存済みの並びが反映されないまま一瞬既定の並びで描画され、
// 0 件のときに「まだ何も無い」のか「絞り込みすぎ」なのかも言えない。
Promise.all([refreshStatus(), setupSort()]).then(runSearch);
