# ADR-0001: ライセンスは MIT

- **状態**: 承認済み
- **日付**: 2026-09-21
- **関連**: D1

## 背景

OSS として公開するにあたりライセンスを決める必要がある。候補は Rust 界隈で慣習的な `MIT OR Apache-2.0`、MIT 単独、GPL-3.0。

## 決定

**MIT License 単独**を採用する。

## 理由

- **利用者にとって最も明快**。File Origin は他プロジェクトに組み込まれるライブラリではなくエンドユーザー向けアプリなので、デュアルライセンスの利点（特許条項を選べる）が効く場面が少ない。
- **ブラウザ拡張側と揃う**。拡張は Chrome Web Store / Firefox Add-ons で配布され、Rust 側と別ライセンスにすると説明が増える。
- **GPL を採らない**理由は、ディストリビューションへの取り込みや、ファイルマネージャなどからの利用を妨げたくないため。File Origin の価値は普及にある。
- 先行事例の [WhereFrom](https://github.com/opsorart/WhereFrom) も MIT。将来コードを参照・引用する場合に摩擦がない。

## 結果

- `LICENSE` を MIT で配置した。著作権表記は `Copyright (c) 2026 File Origin contributors`。
- 依存クレートを追加するときは MIT / Apache-2.0 / BSD 系であることを確認する。GPL / AGPL のクレートは採用しない。
