# 既存 OSS・製品の調査（Prior Art）

> **D2 の成果物。** [README §15 Decision Log](../README.md#15-未決事項decision-log) の D2 に対応する。
> 調査日: **2026-09-21**（Star 数・コミット数などの数値はこの時点のもの）

---

## 0. 結論（先に）

**File Origin が埋めようとしている穴は実在する。** ただし「入手元を記録する」という要素単体には既存事例が多数あり、素朴な差別化はできない。

調査の結果、既存事例は次の 3 つの軸のうち **どれか 1〜2 つしか満たしていない**。

| 軸 | 内容 |
| --- | --- |
| **A. 汎用性** | メディア・論文・Mod などに限定せず、あらゆるファイルを対象にする |
| **B. in-place** | ユーザーのフォルダ構成を変えず、その場にあるファイルを記録する（取り込まない） |
| **C. 移動追跡** | ファイルが移動・リネームされても関連付けを維持する |

**A・B・C を同時に満たす OSS は見つからなかった。** ここが File Origin の立ち位置。

一方で、調査によって **設計上の誤りが 2 件** 見つかった（[§6 設計へのフィードバック](#6-設計へのフィードバック)）。特に Linux の入手元メタデータに関する前提は崩れており、README を修正済み。

---

## 1. 調査方法

- Web 検索（英語）で「file provenance」「download origin」「where from」「source URL tracking」等の語を組み合わせて探索
- GitHub / Chrome Web Store / Firefox Add-ons / AlternativeTo を横断
- 近接領域（メディア管理・研究データ管理・Mod 管理・ダウンローダ）にも範囲を広げた
- OS 組み込みの仕組みは一次情報（Microsoft Learn / freedesktop.org / Bugzilla）で裏を取った

**調査の限界**: 日本語圏・中国語圏のツール、および GitHub 外で配布されている個人製ツールは網羅していない。

---

## 2. OS 組み込みの仕組み（ベースライン）

File Origin の競合ではなく **土台**。どこまで OS が既にやってくれているかを把握しておく。

### 2.1 Windows — Zone.Identifier / Mark of the Web ○ 使える

NTFS の代替データストリーム `<file>:Zone.Identifier` に INI 形式で書かれる。

```ini
[ZoneTransfer]
ZoneId=3
ReferrerUrl=https://example.com/download-page
HostUrl=https://cdn.example.com/files/setup.zip
HostIpAddress=203.0.113.10
```

- 主要ブラウザ（Chrome / Edge / Firefox）とメールクライアントが `IAttachmentExecute` 経由で **自動的に書く**
- `HostUrl`（実際の取得先）と `ReferrerUrl`（人間が見ていたページ）が分かれている
- フォレンジック領域で確立した仕組みで、仕様が安定している

**評価**: Windows では OS メタデータ経路が **実用的に機能する**。File Origin の「拡張機能を入れる前のファイルの救済」はここで成立する。

### 2.2 macOS — kMDItemWhereFroms ○ 使える（今回はスコープ外）

`com.apple.metadata:kMDItemWhereFroms` に URL と referrer の配列が入る。plist 形式。
将来 macOS 対応する際（D4）はここを読めばよい。

### 2.3 Linux — △ **当てにならない**

freedesktop.org の [Common Extended Attributes](https://www.freedesktop.org/wiki/CommonExtendedAttributes/) に `user.xdg.origin.url` / `user.xdg.referrer.url` が **標準として定義されている**。しかし実装状況は悪い。

| 実装 | 状況 |
| --- | --- |
| **Firefox** | × xattr を書かない。代わりに **GVFS メタデータ** の `metadata::download-uri` に書く（[Bug 797349](https://bugzilla.mozilla.org/show_bug.cgi?id=797349) で実装）。xattr 対応要望の [Bug 665531](https://bugzilla.mozilla.org/show_bug.cgi?id=665531) は 10 年以上未解決 |
| **Chrome / Chromium** | △ 一度実装したが **サポートを打ち切った** |
| **wget --xattr** | ○ 書く |
| **curl --xattr** | ○ 書く |

**GVFS メタデータの所在**: `~/.local/share/gvfs-metadata/main.db`（SQLite）。`gio info -a "metadata::*" <file>` で読める。
なお [Bug 1535950](https://bugzilla.mozilla.org/show_bug.cgi?id=1535950) の通り、**プライベートブラウジング中でも記録される**という指摘がある。File Origin がこれを読む際はプライバシー上の配慮が要る。

**評価**: **Linux では xattr だけを見ていると、主要ブラウザからの入手元をほぼ拾えない。**
→ GVFS メタデータの読み取りを追加し、それでも Windows ほどの網羅性は無いと認めるべき。**Linux ではブラウザ拡張（M4）が事実上の必須機能**になる。

---

## 3. 直接の競合

### 3.1 opsorart/WhereFrom — ⭐ 最も近い

> "A Windows-first, local-first file provenance tool that tells you where a file came from."
> https://github.com/opsorart/WhereFrom

| 項目 | 内容 |
| --- | --- |
| 言語 / スタック | C# / .NET 10 |
| プラットフォーム | **Windows のみ**（Windows 11 x64 / NTFS でテスト） |
| 入手元の取得 | **Zone.Identifier ADS の読み取りのみ** |
| インターフェース | **CLI のみ**（`wherefrom.exe` / `open` / `scan` / `debug-zone`） |
| 移動・リネーム追跡 | × **なし**（README に「file-move tracking are not available」と明記） |
| GUI / エクスプローラ統合 | × なし |
| 永続的なデータベース | × なし（都度読むだけ） |
| ブラウザ連携 | × なし |
| ライセンス | **MIT** |
| 活動状況 | Star 0 / Fork 0 / コミット 11 — ごく初期段階 |

**コンセプトの言葉づかいまで File Origin とほぼ同じ**（"local-first file provenance" / "where a file came from"）。
ただし実質は **Zone.Identifier リーダー**であり、File Origin の設計で言えば **M2 相当の範囲に留まる**。

> **示唆**
> - ○ **コンセプトの需要は裏付けられた** — 同じ問題意識を持つ人が他にもいる
> - ○ **差分は明確** — DB による永続化（M1）、移動追跡（M3）、ブラウザ拡張（M4）、GUI（M5）、Linux 対応のすべてが未実装
> - ○ **MIT なので参考にできる** — Zone.Identifier のパース実装（64 KiB の読み取り上限、BOM 検出、エンコーディング処理）は先行事例として学べる
> - △ 名前が近い。`WhereFrom` と `File Origin` の混同を避ける意味でも、機能面での差別化を README で明示しておくのが良い

---

## 4. 隣接領域（部分的に重なる）

### 4.1 hydrus network — B（in-place）を満たさない

- SHA-256 をファイルの主キーとし、タグ・評価・メモ・**known URLs** を紐づける
- ハッシュ一致で重複ダウンロードをスキップし、その際も URL は known URLs に追加する
- 破損チェック時に「失われたファイルの known URLs」を `.txt` に書き出して再取得に回せる

**File Origin との違い**: hydrus は **ファイルを自前のストレージに取り込む（import）**。ユーザーのフォルダ構成は維持されない。また画像・動画などのメディアに強く特化している。
→ **A（汎用性）と B（in-place）を満たさない。** ただし「ハッシュを主キーに URL を紐づける」というデータモデルは File Origin の `origins` テーブルとほぼ同じ発想で、**設計の妥当性を裏付ける**。

### 4.2 git-annex / DataLad — 用途が違う

- `git annex addurl` で入手元 URL を記録。`--file` で既存ファイルの**代替取得元**として URL を登録できる
- `VURL` バックエンドは URL 由来のコンテンツをチェックサムで検証できる
- DataLad の `download-url` はダウンロードと同時に来歴を記録し、`git annex whereis` で入手元を引ける

**File Origin との違い**: git リポジトリの中にあることが前提。日常の `~/Downloads` を管理する道具ではない。研究データ管理の文脈に最適化されている。
→ **来歴追跡の思想としては最も成熟している**ので、用語法（provenance / whereis）は参考になる。

### 4.3 TagSpaces — 入手元記録が主眼ではない

- OSS（Windows / macOS / Linux / Web）、完全オフライン、タグによるファイル整理
- Firefox / Chrome の **Web Clipper 拡張** があり、Web コンテンツをローカルファイルとして収集できる
- **in-place**（ファイルを移動させない）で、サイドカーファイルにメタデータを持つ

**File Origin との違い**: 主眼は「タグ付け」であって「入手元の自動記録」ではない。ダウンロード全般を自動で捕捉する仕組みは持たない。
→ **B（in-place）のお手本。** サイドカー方式 vs 中央 DB 方式という設計選択の参考になる（File Origin は中央 DB を選択済み）。

### 4.4 Eagle — 商用・非 OSS

- デザイン素材管理ツール。タグ・色・**URL ソース**などでフィルタできる
- Windows / macOS のみ。**Linux 非対応**、**クローズドソース**、買い切り有料

**File Origin との違い**: OSS ではない。素材管理に特化。
→ 「入手元 URL で素材を検索したい」という需要が **商用として成立している** ことの証拠。

### 4.5 ドメイン特化ツール群 — A（汎用性）を満たさない

| ツール | 領域 | 入手元の扱い |
| --- | --- | --- |
| **Zotero** | 論文・PDF | 取得元 URL・DOI を保持。文献管理に特化 |
| **Calibre** | 電子書籍 | 取得元を保持。書籍に特化 |
| **Vortex / Mod Organizer 2** | ゲーム Mod | Nexus Mods と API 連携し、mod の入手元と更新を管理。ゲームごとに閉じる |
| **gallery-dl / yt-dlp** | 画像・動画 | サイドカー JSON（`--write-info-json`）に入手元を書く。`--download-archive` で取得済みを記録 |

いずれも **その領域の中では File Origin より優れている**。File Origin はこれらを置き換えるものではなく、**領域の外側にあるすべてのファイル**を拾う。

> README §2 の想定ユーザーに「ゲーム・Mod を導入する人」があるが、**Nexus Mods 経由なら Vortex の方が優れている**。File Origin の価値は「Nexus 以外から拾ってきた Mod」「配布サイトが消えた Mod」にある。想定ユーザーの説明はこの点を踏まえて書き直す余地がある。

### 4.6 ブラウザのダウンロード履歴 — ベースライン

Chrome / Firefox の履歴が、ほとんどのユーザーにとっての現状の答え。

**限界**（＝ File Origin の存在理由）:
- ファイルを**移動・リネームすると紐付けが切れる**
- 履歴の保持期間が有限で、消去すると失われる
- ブラウザをまたげない・プロファイルをまたげない
- ファイル側から逆に引けない（「このファイルの出所は？」に答えられない）

---

## 5. 参考になる実装

競合ではないが、実装面で学べるもの。

### 5.1 AllTheThings — ⭐ スタック選定の裏付け

> "A voidtools Everything clone — instant NTFS filename search for Windows, built in **Rust + Tauri**"
> https://github.com/xin-521/AllTheThings

- NTFS の **MFT を直読み**してボリュームを数秒でインデックス
- **USN Change Journal を tail** して create / delete / **rename** をライブ反映
- ボリュームごとの `UsnWatcher` が、ジャーナルが省略するサイズ・タイムスタンプを MFT 再読で補う

→ **File Origin の Windows 側 watcher（M3）の参考実装として最良。** Rust + Tauri という選定の妥当性も裏付けられた。

### 5.2 Everything（voidtools）— USN の運用知見

- MFT 直読み + USN Journal tail という構成の原典
- **USN Journal が保持するのは約 1 週間分** という運用上の重要な制約

### 5.3 gyng/save-in — ブラウザ拡張の参考

- MV3 対応の WebExtension（Firefox 140+ / Chrome 123+）
- **referrer URL・ホスト名・ルートドメイン・MIME・ブラウザが解決した最終ファイル名** でルーティングできる

→ 拡張が `downloads` API からどこまでの情報を取れるかの実例。File Origin の拡張（M4）が取得すべき項目の参考になる。

### 5.4 gvfs-meta-explorer — GVFS メタデータの読み方

https://github.com/emanuele-f/gvfs-meta-explorer — Linux で `metadata::download-uri` を読むために必要。

---

## 6. 設計へのフィードバック

調査の結果、**README の記述に誤りが 2 件** 見つかった。いずれも修正済み。

### F1. △ Linux の入手元メタデータ — 前提が崩れていた

**誤り**: README は `user.xdg.origin.url` を「Firefox / wget --xattr / curl --xattr が書く」としていた。
**事実**: **Firefox は xattr を書かない**（GVFS メタデータに書く）。**Chrome は実装後に撤回した**。実際に xattr を書くのは wget / curl の `--xattr` オプションくらい。

**対応**:
- `OriginMetadata` の Linux 実装に **GVFS メタデータ（`metadata::download-uri`）読み取りを追加**
- Linux の OS メタデータ経路の確度を `high` から下げ、**取得できないことが普通**という前提に立つ
- **Linux ではブラウザ拡張（M4）が必須**であることを README に明記
- プライベートブラウジング中の記録を読む可能性があるため、GVFS 読み取りは**オプトイン**にする

### F2. △ USN Change Journal — 「完全に追える」は誤り

**誤り**: README は USN Journal について「停止中の変更も**完全に**追える」と書いていた。
**事実**: 2 つの制約がある。

1. **保持期間が有限** — 既定で約 1 週間分。それより古いカーソルからは読み直せない
2. **Administrator 権限が必要** — [Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal) の通り、USN の操作には Administrators グループのメンバーであることが要る。ボリュームハンドル（`\\.\C:`）を開く時点で昇格が必要

これは README §14 の「**管理者 / root を要求しない**」という方針と正面から衝突していた。

**対応**: USN を **Linux の `fanotify` と同格の「任意の特権機能」** として位置づけ直した。結果として両 OS が対称になる。

| | 特権あり（完全） | 特権なし（既定） |
| --- | --- | --- |
| Windows | USN Change Journal | 差分スキャン |
| Linux | fanotify | 差分スキャン |

**既定では両 OS とも差分スキャン**で動き、特権を与えれば精度が上がる。この方が設計方針 P4（劣化して動く）とも整合する。

### F3. ○ 中央 DB + in-place 方式は妥当

hydrus（import 方式）と TagSpaces（サイドカー方式）を比べると、File Origin の「**in-place + 中央 SQLite**」は妥当な選択。

- import 方式はユーザーのフォルダを支配してしまい、想定ユーザー（「PC を日常的に利用する人」）に受け入れられにくい
- サイドカー方式はファイル移動時に一緒に動く利点があるが、ファイルが 2 倍に増え、移動追跡・横断検索が弱い

### F4. ○ 想定ユーザーの記述を見直す余地（D8 で反映済み）

Mod 利用者には Vortex、論文には Zotero、電子書籍には Calibre がある。
File Origin は**それらの領域の外にあるファイル**にこそ価値がある。README §2 をこの立ち位置で書き直した（[D8](#7-decision-log-への追加)、完了）。

---

## 7. Decision Log への追加

調査から新たに生じた論点。

| # | 論点 | 内容 |
| --- | --- | --- |
| **D8** | 想定ユーザーの再定義 | ドメイン特化ツール（Vortex / Zotero / Calibre）との棲み分けを README §2 に反映するか |
| **D9** | GVFS メタデータ読み取り | Linux で `metadata::download-uri` を読むか。プライベートブラウジングの記録を含むためオプトイン前提 |
| **D10** | `WhereFrom` との関係 | 名称・コンセプトが近い。README で差分を明示するか、あるいは協調の可能性を探るか |

---

## 8. 機能比較表

| | 入手元の<br/>自動記録 | 移動<br/>追跡 | 対象 | 方式 | Win | Linux | OSS | ローカル<br/>完結 |
| --- | :---: | :---: | --- | --- | :---: | :---: | :---: | :---: |
| **File Origin**（目標） | ○ | ○ | 汎用 | in-place | ○ | ○ | ○ MIT | ○ |
| WhereFrom | △ ADS のみ | × | 汎用 | 読むだけ | ○ | × | ○ MIT | ○ |
| hydrus network | ○ | ○ | メディア | **import** | ○ | ○ | ○ | ○ |
| git-annex / DataLad | ○ | ○ | リポジトリ内 | git 管理下 | △ | ○ | ○ | ○ |
| TagSpaces | × | △ | 汎用 | in-place | ○ | ○ | ○ | ○ |
| Eagle | ○ | ○ | 素材 | **import** | ○ | × | × | ○ |
| Zotero | ○ | ○ | 論文 | import | ○ | ○ | ○ | △ |
| Vortex / MO2 | ○ | ○ | Mod | import | ○ | △ | ○ | △ |
| gallery-dl / yt-dlp | ○ | × | 自分の DL のみ | サイドカー | ○ | ○ | ○ | ○ |
| ブラウザ履歴 | ○ | × | 自分の DL のみ | 履歴 DB | ○ | ○ | △ | ○ |
| OS メタデータのみ | △ | × | 汎用 | ファイル付随 | ○ | × | — | ○ |

**空白地帯**: 「汎用 × in-place × 移動追跡 × Win/Linux 両対応 × OSS」を満たす行は File Origin だけ。

---

## 9. 出典

- [opsorart/WhereFrom](https://github.com/opsorart/WhereFrom) — Windows-first, local-first file provenance tool
- [Mark of the Web — Wikipedia](https://en.wikipedia.org/wiki/Mark_of_the_Web)
- [Forensic Analysis of the Zone.Identifier Stream — Digital Detective](https://www.digital-detective.net/forensic-analysis-of-zone-identifier-stream/)
- [Hunting tip of the month: Browser downloads — Microsoft Community Hub](https://techcommunity.microsoft.com/blog/microsoftdefenderatpblog/hunting-tip-of-the-month-browser-downloads/220454)
- [FSCTL_READ_USN_JOURNAL — Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal)
- [Guidelines for extended attributes — freedesktop.org](https://www.freedesktop.org/wiki/CommonExtendedAttributes/)
- [Bug 665531 — [Linux] Store that file was downloaded from the Internet (Extended Attribute user.xdg.origin.url)](https://bugzilla.mozilla.org/show_bug.cgi?id=665531)
- [Bug 797349 — Save originating URI in GIO for downloaded files](https://bugzilla.mozilla.org/show_bug.cgi?id=797349)
- [Bug 1535950 — On Linux the download URI is saved to GVFS/GIO metadata even in private browsing](https://bugzilla.mozilla.org/show_bug.cgi?id=1535950)
- [emanuele-f/gvfs-meta-explorer](https://github.com/emanuele-f/gvfs-meta-explorer)
- [hydrus network — FAQ](https://hydrusnetwork.github.io/hydrus/faq.html)
- [git-annex-addurl](https://git-annex.branchable.com/git-annex-addurl/)
- [Basic provenance tracking — The DataLad Handbook](https://handbook.datalad.org/en/inm7/usecases/provenance_tracking.html)
- [TagSpaces](https://github.com/tagspaces/tagspaces)
- [Eagle Alternatives — AlternativeTo](https://alternativeto.net/software/eagle-cool/)
- [xin-521/AllTheThings](https://github.com/xin-521/AllTheThings) — Everything clone in Rust + Tauri
- [USN Journal — Wikipedia](https://en.wikipedia.org/wiki/USN_Journal)
- [FAQ — voidtools](https://www.voidtools.com/faq/)
- [gyng/save-in](https://github.com/gyng/save-in)
- [XATTR Command — ss64 (macOS)](https://ss64.com/mac/xattr.html)
- [Download Management — Nexus-Mods/Vortex (DeepWiki)](https://deepwiki.com/Nexus-Mods/Vortex/6.3-download-management)
