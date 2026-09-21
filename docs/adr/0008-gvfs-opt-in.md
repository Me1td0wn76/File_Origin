# ADR-0008: GVFS メタデータは読むが既定 OFF

- **状態**: 承認済み
- **日付**: 2026-09-21
- **関連**: D9、[prior-art.md §2.3](../prior-art.md)

## 背景

Linux の入手元メタデータは期待通りに機能しない。freedesktop.org は `user.xdg.origin.url` を標準として定義しているが、**Firefox は xattr を書かず**（[Bug 665531](https://bugzilla.mozilla.org/show_bug.cgi?id=665531) は 10 年以上未解決）、**Chrome は実装後に撤回した**。実際に書くのは `wget --xattr` / `curl --xattr` 程度。

一方 Firefox は GVFS メタデータの `metadata::download-uri` に書いている（[Bug 797349](https://bugzilla.mozilla.org/show_bug.cgi?id=797349)）。実体は `~/.local/share/gvfs-metadata/main.db`。

ここを読まないと、Linux では「拡張導入前のファイルを救済する」機能がほぼ成立しない。

## 決定

**GVFS メタデータの読み取りを実装する。ただし既定 OFF のオプトインとする。**

有効化するときは、何が読まれるかを明示して同意を取る。

## 理由

- **Linux で OS メタデータ経路を成立させる唯一の手段**。これがないと Linux ユーザーにとって M2 の価値がほぼゼロになる。
- **しかしプライバシー上の懸念がある**。[Bug 1535950](https://bugzilla.mozilla.org/show_bug.cgi?id=1535950) の通り、**プライベートブラウジング中のダウンロードも GVFS に記録される**。ユーザーが「記録されない」と思っていたものを File Origin が拾い、永続化してしまう。
- ユーザーが意識せずこれを有効にできてしまう設計は、「ローカル完結・プライバシー重視」を掲げるプロジェクトとして筋が通らない。**黙って読まない**ことが重要。

## 結果

- `OriginSource::Gvfs` を追加し、確度は `medium`（xattr の `high` より低い）。ブラウザやプロファイルの情報が取れず、ファイルとの対応も間接的なため。
- 設定で明示的に有効化する。`fo scan` に `--with-gvfs` 相当のフラグを用意する。
- **有効化時に警告を出す**: 「プライベートブラウジング中のダウンロード記録を含む可能性があります」。
- `fo doctor` は、無効時に「GVFS メタデータの読み取りは無効（オプトイン）」と表示し、存在自体は伝える。隠すのではなく、選ばせる。
