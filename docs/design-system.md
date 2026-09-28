# デザインシステム

File Origin の GUI（Tauri / `gui/web`）のためのデザインシステム。開発者ツール寄りの、ダーク基調・高密度な画面で、ダウンロードしたファイルの **入手元・来歴** を見せる。ファイル名と同じ強さで URL を見せ、機械が書いた値はすべて等幅で出す。

実体は `assets/design-system/` にあり、`scripts/sync-design-system.sh` が `gui/web/tokens.css` と
`gui/web/components.css` を作る。`gui/web` 側の 2 枚は取り込み物なので手で直さない。
画面を直すときは、まずこの文書を読む。

## 原則

1. **入手元が主役。** 一覧でも詳細でも「ファイル名 → 入手元 → 確度」の順。URL のホストは `url` の色、パスは `ink` / `ink-muted`。URL は省略記号で切らずに折り返す（一覧の 1 行表示だけは例外）。
2. **経路と確度を必ず並べる（P5）。** 入手元を出す所には SourceTag と ConfidenceMeter を対で置く。確度は記録された値のまま出し、盛らない。
3. **状態は記号と語で。** ○ 良好 / △ 要確認・制約あり / × 失敗・利用不可（`fo doctor` と同じ記号）。`ok` と `danger` は色相しか違わないので、色だけで状態を伝えない。
4. **劣化を隠さない（P4・§7.3）。** 特権が無い・OS が対応しない機能は △ で出し、既定の代替（「起動時の差分スキャンで補正します」）を一文添える。対称なふりをしない。
5. **読み取りのみを言葉で約束する（P6）。** 取り込み・外す・消す操作の近くに「ファイルは移動もコピーもされません」「ファイル自体は残ります」を書く。「削除」という語はファイルに使わない。
6. **CLI と同じ操作（P2）。** QueryBar のキーは `fo search` のフラグと 1 対 1。詳細・取り込み・診断には同じ操作の CommandLine を添える。
7. **ローカル完結（P3）。** 外部のフォント・スクリプト・画像を読まない。CSP は `default-src 'self'` のまま。プライバシーに触れる設定には同意ダイアログを挟む。

## 文言

- です・ます調で短く。感嘆符・絵文字は使わない。ボタンは動詞で終える（「取り込む」「コピー」「インストール」）。
- 用語を固定する：入手元（origin）・参照元（referrer）・取得日時（acquired_at）・経路（source）・確度（confidence）・来歴・見失い中（missing）・取り込み（scan）・再スキャン（rescan）・監視フォルダ（scan_roots）・デーモン。
- 確度の語は 確定 / 高 / 中 / 低、経路の語は ブラウザ拡張 / `Zone.Identifier` / 拡張属性 / `GVFS` / 手動登録 / ブラウザ履歴、状態の語は あり / 見失い中 / 削除済み（現行 `app.js` と同じ）。技術名は訳さず `<code>` で。
- 日時は詳細で `YYYY-MM-DD HH:mm`、一覧で `MM-DD HH:mm`（今年でなければ年を付ける）。サイズは `fmtSize` の丸め（`342 MB`・`2.1 MB`）。件数は 3 桁区切り（`1,284 件`）。
- 空状態と注意書きは「何が起きているか」＋「どうすれば戻るか」の 2 文まで。

## 色

- テーマは `dark` が第一（既定）、`light` が第二。既定は OS に合わせ、`html[data-theme]` を `matchMedia('(prefers-color-scheme: light)')` で切り替える。設定で固定もできる。
- 面は 5 段：`bg`（ウィンドウ・一覧・サイドバー・値の井戸）→ `surface`（ツールバー・詳細・ダイアログ）→ `surface-2`（入手元カード・ボタン・ホバー）→ `surface-3`（現在地・押下）。区切りは `line`、操作部品の輪郭は `line-strong`（3:1 以上）。
- `accent` は操作と選択にだけ使う：主ボタン・選択行の地 `accent-soft`・チップ・進捗・取得ノード。主ボタンの文字は `on-accent`（白決め打ちにしない）。
- `ok` / `warn` / `danger` は状態専用。淡い地（`*-soft`）の上に同じ色の文字を載せる。装飾に使わない。
- `ink` と `ink-muted` はすべての面で 4.5:1 以上。`accent` と状態色も文字として全面で 4.5:1 以上（両テーマで確認済み）。

## 文字

- UI は OS の日本語 UI フォント（`--font-sans`：Segoe UI / Yu Gothic UI、Linux は Noto Sans CJK JP）。CJK フォントは同梱しない。
- 機械が書いた値 — URL・パス・SHA-256・識別子・日時・サイズ・件数・検索クエリ・コマンド — は `--font-mono`（IBM Plex Mono）。人が読む文は sans。
- IBM Plex Mono（SIL OFL 1.1）は woff2 を `gui/web/fonts/` に同梱し、`@font-face` で `'self'` から読む。Google Fonts を実行時に読まない。**未同梱** — 今は `--font-mono` の次点（Cascadia Mono / DejaVu Sans Mono）に落ちている。
- 基準は `body` 13px/20px。一覧のファイル名は `body-strong`、詳細の名前とダイアログの題は `title`、セクション見出しは `label`（`ink-muted`）。`display` は空状態だけ。

## 余白・形・影

- 4px 基準（`space-1`〜`space-8`）。高密度なので行・カードの内側は `space-3`、ペインの内側は `space-4`。
- 角は小さく：チップ・Badge は `radius-xs`、ボタン・入力は `radius-sm`、カード・ダイアログは `radius-md`。丸はステータスの点とスイッチだけ。
- 段差は色と線で出す。影（`shadow-pop`）はダイアログ・メニュー・ポップオーバー・トーストだけ。
- 左端に色帯を付けたカード・行は作らない。選択は面の色で示す。

## レイアウト

| 領域 | 寸法 | 地 |
| --- | --- | --- |
| Toolbar | 高さ `toolbar-h` 44px | `surface` |
| Sidebar（ビュー・ホスト） | 幅 `sidebar-w` 184px | `bg` |
| 一覧（ListBar ＋ FileRow） | 最小 320px、行高 `row-h` 48px | `bg` |
| 詳細 | 46%・最小 420px | `surface` |
| StatusBar | 高さ `statusbar-h` 24px | `surface` |

- ウィンドウは既定 1100×720・最小 720×480（`tauri.conf.json`）。1000px 未満でサイドバーを畳み、820px 未満で一覧と詳細を切り替え表示にする（`Esc` で一覧へ）。
- 設定（診断・監視フォルダ・入手元の経路・ブラウザ連携・データ・一般）は一覧の上に全面で重ね、`Esc` で閉じる。
- 実装済みの画面は 2 ペイン（サイドバーは facet 集計が入るまで畳んである。`fo-panes.no-side`）。

## 状態と操作

- ホバーは `surface-2`、選択は `accent-soft`（`aria-selected`）、現在地は `surface-3`（`aria-current`）。
- フォーカスは `focus` の 2px 実線（`outline-offset: 1px`、一覧行は内側）。全面で 3:1 以上。消さない。
- 主な操作はキーで届く：`Ctrl K` / `/` 検索、`↑` `↓` 行移動、`Enter` 詳細、`Ctrl C` 入手元 URL をコピー、`Ctrl I` 取り込み、`Ctrl ,` 設定、`Esc` 閉じる。
- 動きは最小限：メニューとダイアログの出入りに 120ms のフェードだけ。`prefers-reduced-motion` では無くす。
- 長い処理（取り込み・SHA-256）は止めずにバックグラウンドへ回せるようにし、進捗を StatusBar に残す。

## アイコン

- 独自の 16px グリッド・線幅 1.5・端と角は丸のアイコン 29 個（`assets/design-system/icons/`）。単色で、UI では `fo-i fo-i-<name>` の CSS マスクとして `currentColor` で塗る。
- `assets/design-system/bundle.css` はアイコンを `data:` URI で持つ。そのまま使うなら CSP に `img-src 'self' data:` を足すか、`assets/design-system/icons/*.svg` を `gui/web/icons/` に置いて `--i-<name>` を `url("icons/<name>.svg")` に差し替える。
- アイコンは語の代わりにしない。アイコンだけのボタンには `aria-label` と `title` を付ける。絵文字は使わない。
- ツールバーの名前は文字の「File Origin」だけ。アプリアイコン（`assets/icon/`、[ブランド](brand.md)）は OS のタスクバーとインストーラ用で、画面の中には置かない。

## 実装

- `assets/design-system/bundle.css` は素の CSS。すべて `fo-` 接頭辞で `assets/design-system/tokens.css` の変数を読む。バンドラ無しの現行 `gui/web` にそのまま入り、React（D5）でも同じ className が使える。
- `pv`・`pv-col`・`pv-screen` はプレビュー用の補助なので、アプリには持ち込まない。
- 型スタイル（`title`・`body`・`mono` など）は `tokens.css` のクラスとしても使える。ただし接頭辞が無いので、アプリ側では `fo-` の部品を優先する。
