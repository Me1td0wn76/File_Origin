# ADR-0003: OS 差は fo-platform 1 層に閉じ込める

- **状態**: 承認済み
- **日付**: 2026-09-21

## 背景

Windows と Linux では、ファイルの安定識別子・入手元メタデータ・監視 API・IPC・自動起動・データ配置先のすべてが異なる。これを各所で `#[cfg]` 分岐すると、コアが OS を知り、テストが実機依存になり、新 OS 対応のたびに全体を触ることになる。

## 決定

OS 固有の `#[cfg]` と OS 固有クレート（`windows` / `nix` / `rustix` / `xattr` / `libc` 等）への依存は、**`crates/fo-platform` の中にしか置かない**。他のクレートは `Platform` trait だけに依存する。

この不変条件は `scripts/arch-guard.sh` で機械的に検査し、CI の必須ジョブとする。

## 理由

- **コアのテストが OS に依存しなくなる**。`fo-platform::mock` を挿せば、Windows の開発機で Linux 相当の挙動もテストできる。
- **新 OS 対応がディレクトリ 1 つで済む**。macOS 対応（D4）は `fo-platform/src/macos/` を足すだけで、他クレートは変更不要。
- **非対称性が設計時に露出する**。両 OS を同じ trait に通す過程で「Windows にはあるが Linux にはない」が必ず表面化する。これを暗黙にすると後で壊れる。

## 結果

- `Platform` trait を唯一のエントリポイントとし、`fo_platform::current()` が実装を返す。`current()` が **workspace 内で唯一の cfg 分岐点**になる。
- 能力差は `Capabilities` 型で表現し、`fo doctor` が実行環境の実力を表示する。
- 規約を口約束にしないため、検査スクリプトと Claude Code 用 skill（`.claude/skills/platform-layer/`）を同時に用意した。
- **コスト**: 抽象化の分だけコード量が増える。両 OS で同一コードが動く処理（`std::fs` で足りるもの）は trait にしない、という線引きで抑える。
