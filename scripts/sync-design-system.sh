#!/usr/bin/env bash
# デザインシステムを GUI に取り込む。
#
#   bash scripts/sync-design-system.sh
#
# 出所は assets/design-system/。gui/web/tokens.css と gui/web/components.css は
# その取り込み物で、手で直す場所ではない。直すのは assets/design-system/ のほう。
# 取り込みを手作業にしないのは、手で写すとどちらが新しいのか分からなくなるから。
#
# 決まりごとは docs/design-system.md。
#
# 取り込むのは 2 つだけ:
#   tokens.css  → gui/web/tokens.css      （そのまま）
#   bundle.css  → gui/web/components.css  （プレビュー専用クラスを落とす）
#
# アイコンは bundle.css の中に data: URI で入っている。別ファイルは要らないが、
# そのぶん tauri.conf.json の CSP に `img-src 'self' data:` が要る。

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
src="$root/assets/design-system"
dst="$root/gui/web"

for f in tokens.css bundle.css; do
  if [ ! -f "$src/$f" ]; then
    echo "デザインシステムが欠けています: $src/$f" >&2
    exit 1
  fi
done

cp "$src/tokens.css" "$dst/tokens.css"
echo "○ tokens.css"

# プレビュー専用の .pv / .pv-col / .pv-screen はアプリでは使わない。
# 部品カードを並べるためだけのもので、持ち込むと紛らわしい。
{
  cat <<'HEADER'
/* File Origin — デザインシステムの部品スタイル。
 *
 * このファイルは scripts/sync-design-system.sh が作る。手で直さない。
 * 直すのは assets/design-system/bundle.css のほう。決まりごとは docs/design-system.md。
 * プレビュー専用の .pv / .pv-col / .pv-screen だけ落としてある。
 *
 * 色・寸法の変数は tokens.css が持つ。ここは形だけ。
 *
 * アイコンは data: URI の SVG を CSS マスクとして使う。単色なので currentColor に従い、
 * 外部への取得も起きない。そのぶん tauri.conf.json の CSP に img-src 'self' data: が要る
 * （マスク画像は img-src で判定される）。
 */
HEADER
  awk '/preview helpers \(cards only\)/{skip=1} /^\/\* -+ Icons:/{skip=0} !skip' \
    "$src/bundle.css" | tail -n +5
} > "$dst/components.css"
echo "○ components.css"

# 落とし忘れの検査。プレビュー用のクラスが残ると、アプリ側で当たる恐れがある。
if grep -qE '^\.pv' "$dst/components.css"; then
  echo "× プレビュー専用クラスが残っています" >&2
  exit 1
fi

echo "取り込みました。cargo build -p fo-gui で反映されます。"
