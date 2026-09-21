# Architecture Decision Records

設計上の決定と、その理由の記録。README の [Decision Log](../../README.md#15-未決事項decision-log) から参照される。

新しい決定を足すときは連番で追加し、既存の ADR を書き換えない。決定が覆ったときは新しい ADR を書き、古い方の状態を「置き換え済み」にする。**理由が残っていることに価値がある**ので、間違っていた決定も消さない。

| # | 決定 | 状態 | 関連 |
| --- | --- | --- | --- |
| [0001](0001-license-mit.md) | ライセンスは MIT | 承認済み | D1 |
| [0002](0002-rust-tauri-stack.md) | Rust + Tauri を採用する | 承認済み | — |
| [0003](0003-platform-abstraction.md) | OS 差は fo-platform 1 層に閉じ込める | 承認済み | — |
| [0004](0004-in-place-central-db.md) | in-place + 中央 SQLite（import / サイドカーを採らない） | 承認済み | D2 |
| [0005](0005-privileged-features-optional.md) | 特権が要る機能は任意の高速化として扱う | 承認済み | D2 (F2) |
| [0006](0006-no-db-encryption-v1.md) | v1 では DB を暗号化しない | 承認済み | D3 |
| [0007](0007-hashing-strategy.md) | SHA-256 は全体を、遅延して計算する | 承認済み | D7 |
| [0008](0008-gvfs-opt-in.md) | GVFS メタデータは読むが既定 OFF | 承認済み | D9 |
