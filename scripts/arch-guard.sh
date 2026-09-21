#!/usr/bin/env bash
#
# arch-guard — File Origin のアーキテクチャ不変条件を検査する
#
# 不変条件: OS 固有のコードと依存は fo-platform クレートにしか存在しない。
#           （README「設計方針 P1」/「レイヤーアーキテクチャ」を参照）
#
# CI からも手元からも同じものを実行する。Windows では Git Bash で動く。
#
#   使い方:  ./scripts/arch-guard.sh
#   終了コード: 0 = 違反なし / 1 = 違反あり

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PLATFORM_CRATE="crates/fo-platform"
VIOLATIONS=0

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
dim()   { printf '\033[2m%s\033[0m\n' "$*"; }

# ripgrep があれば使い、無ければ grep にフォールバックする。
# どちらの出力も「パス:行番号:本文」に揃え、パス区切りは / に正規化する。
# （Windows の ripgrep は \ を出すため、除外パターンがそのままでは効かない）
if command -v rg >/dev/null 2>&1; then
    SEARCH() {  # SEARCH <正規表現> <ディレクトリ> <ファイルglob>
        rg -n --no-heading -e "$1" "$2" --glob "$3" 2>/dev/null | tr '\\' '/'
    }
else
    SEARCH() {
        find "$2" -name "$3" -type f -exec grep -EnH -e "$1" {} + 2>/dev/null | tr '\\' '/'
    }
fi

if [ ! -d crates ]; then
    dim "crates/ がまだありません。検査をスキップします。"
    exit 0
fi

# --- 1. OS 固有の cfg 属性 -------------------------------------------------
# fo-platform の外に OS 分岐があると、コアが OS を知ってしまう。
# #[cfg(test)] や feature フラグは対象外。

echo "[1/2] OS 固有 cfg の検査..."

# #[cfg(windows)] / #[cfg(unix)] / #[cfg(target_os = "linux")] / cfg!(windows) /
# not(...) や any(...) で包まれたものも拾う。cfg( の括弧はリテラルなのでエスケープが要る。
CFG_PATTERN='cfg!?\([^)]*\b(windows|unix)\s*[,)]|cfg!?\([^=]*\b(target_os|target_family)\s*='

CFG_HITS="$(
    SEARCH "$CFG_PATTERN" crates '*.rs' \
        | grep -v "^${PLATFORM_CRATE}/" \
        || true
)"

if [ -n "$CFG_HITS" ]; then
    red "✗ fo-platform の外で OS 固有の cfg が使われています:"
    echo "$CFG_HITS" | sed 's/^/    /'
    echo
    dim "  → この分岐は ${PLATFORM_CRATE} の trait の裏に移してください。"
    dim "    呼び出し側は Platform trait 経由で能力を問い合わせます。"
    echo
    VIOLATIONS=$((VIOLATIONS + 1))
else
    green "✓ OS 固有 cfg は fo-platform のみ"
fi

# --- 2. OS 固有クレートへの依存 -------------------------------------------
# Cargo.toml の依存に OS 固有クレートが現れたら、そのクレートは
# 暗黙に OS を知っている。

echo "[2/2] OS 固有クレート依存の検査..."

OS_CRATES='^\s*(windows|windows-sys|windows-targets|nix|rustix|xattr|winapi|libc|inotify|fsevent[a-z-]*)\s*='

DEP_HITS="$(
    SEARCH "$OS_CRATES" crates 'Cargo.toml' \
        | grep -v "^${PLATFORM_CRATE}/" \
        || true
)"

if [ -n "$DEP_HITS" ]; then
    red "✗ fo-platform の外で OS 固有クレートに依存しています:"
    echo "$DEP_HITS" | sed 's/^/    /'
    echo
    dim "  → その機能を ${PLATFORM_CRATE} の trait として公開し、"
    dim "    呼び出し側は trait だけに依存してください。"
    echo
    VIOLATIONS=$((VIOLATIONS + 1))
else
    green "✓ OS 固有クレート依存は fo-platform のみ"
fi

echo
if [ "$VIOLATIONS" -eq 0 ]; then
    green "arch-guard: 違反なし"
    exit 0
else
    red "arch-guard: ${VIOLATIONS} 件の違反"
    dim "背景: README「6. レイヤーアーキテクチャ」および「7. プラットフォーム抽象化層」"
    exit 1
fi
