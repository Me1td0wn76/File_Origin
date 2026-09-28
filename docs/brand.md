# アイコンとバナー

実体は `assets/icon/` と `assets/banner/`。配布先（Tauri・拡張・README）へは複製してあるので、
**直すときは `assets/` の原本を直してから配る**。[デザインシステム](design-system.md)の色をそのまま使っている。

## アイコン

「F」の字を来歴のレール（幹・枝・ノード）で描いたマーク。下の点が入手元、上へ伸びる幹が来歴、
枝の先が移動・コピー先。地は `accent`（#5cb8f2）、線とノードは `bg`（#0e1013）。
テーマに左右されない 1 色の組なので、明るいタスクバーでも暗いタスクバーでも同じものを使う。

| 原本 | 用途 |
| --- | --- |
| `assets/icon/file-origin-icon.svg` | 原本（512 グリッド）。48px 以上はここから |
| `assets/icon/file-origin-icon-16.svg` `-24.svg` `-32.svg` | 画素に合わせた小サイズ専用版（線を太く、リングを塗りに） |
| `assets/icon/file-origin-mark.svg` | 地なしのマーク。`currentColor` で塗られるので、HTML にインラインで置く |
| `assets/icon/file-origin-icon-1024.png` | `cargo tauri icon` の元画像、ストア掲載用 |

配布先：

| 置き場所 | 中身 |
| --- | --- |
| `gui/src-tauri/icons/` | Tauri の 5 ファイル（`icon.ico` は 16/24/32/48/64/256 入り） |
| `extension/{shared,chrome,firefox}/icons/` | ブラウザ拡張用 16/32/48/128。manifest の `icons` と `action.default_icon` が参照する |

画面の中にはアプリアイコンを置かない。ツールバーの名前は文字の「File Origin」だけ
（デザインシステムの「アイコン」節）。

## バナー

デザインシステムのカバーと同じ構図。文字は IBM Plex Mono / Noto Sans CJK JP をアウトライン化して
あるので、フォントが無い環境でも同じに見える。

| ファイル | 大きさ | 用途 |
| --- | --- | --- |
| `assets/banner/banner-dark.svg` `.png` | 960×288（PNG は 2 倍の 1920×576） | README の先頭 |
| `assets/banner/banner-light.svg` `.png` | 同上 | README の先頭（ライト表示用） |
| `assets/banner/social-preview-dark.png` `-light.png` | 1280×640 | GitHub の Settings → Social preview |

README の先頭は `<picture>` で、閲覧者のテーマに合わせて切り替えている。

## フォント

バナーの名前は IBM Plex Mono Medium（SIL Open Font License 1.1、© IBM Corp.）、
説明文は Noto Sans CJK JP（SIL OFL 1.1）。どちらもアウトライン化して使っているので、
配布物にフォントそのものは含まれない。
