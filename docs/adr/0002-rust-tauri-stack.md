# ADR-0002: Rust + Tauri を採用する

- **状態**: 承認済み
- **日付**: 2026-09-21

## 背景

Windows / Linux 両対応のデスクトップアプリを作る。ファイル監視・ハッシュ計算・OS 固有メタデータの読み取りといった、OS API に深く触る処理が中心になる。候補は Rust + Tauri / Go + Wails / TypeScript + Electron / Python。

## 決定

**Rust + Tauri v2** を採用する。コアはライブラリとして切り出し、CLI・GUI・Native Messaging ホストがその上に載る 3 層構成とする。

## 理由

- **OS 固有 API を trait で抽象化しやすい**。`windows-rs` と `nix` があり、`#[cfg]` による切り分けが言語機能として自然。これが [ADR-0003](0003-platform-abstraction.md) の前提になる。
- **単一バイナリで配布でき、ランタイム不要**。エンドユーザーに .NET や Python の導入を要求しない。
- **Native Messaging ホストを同じバイナリで兼ねられる**。ブラウザが頻繁に spawn する短命プロセスなので、起動が速く依存のない実行ファイルが要る。
- **GUI は OS 標準 WebView を使う**（Windows: WebView2 / Linux: WebKitGTK）。Electron と違い Chromium を同梱しないため配布サイズが小さい。
- **先行事例がある**。[AllTheThings](https://github.com/xin-521/AllTheThings) は Rust + Tauri で NTFS の MFT 直読みと USN Journal の tail を実装しており、この用途で成立することが実証されている。

## 却下した選択肢

- **Go + Wails** — クロスコンパイルは容易だが、OS 固有 API が cgo / syscall になり記述が煩雑。trait に相当する抽象化も弱い。
- **TypeScript + Electron** — UI 開発は最速で拡張と言語が揃うが、バイナリが重く、ファイル監視とハッシュ計算の性能、ネイティブ API への到達性で不利。
- **Python** — プロトタイプは最速だが、配布（PyInstaller 等）と常駐性能が弱く、OSS としての導入体験が落ちる。

## 結果

- Cargo workspace で `fo-core` / `fo-platform` / `fo-store` / `fo-watcher` / `fo-ipc` / `fo-daemon` / `fo-cli` / `fo-nativehost` に分割する。
- GUI フロントエンドの選定（D5）は M5 着手時まで保留する。
