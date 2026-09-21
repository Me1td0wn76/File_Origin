//! 標準出力への書き込み。
//!
//! `println!` は書き込みに失敗すると panic する。`fo search | head` のように
//! 読み手が先に終了するのはパイプの正常な使い方なので、panic で終わるのは誤り
//! （終了コード 101 になり、シェルのスクリプトから見ると異常終了に見える）。
//!
//! そこで書き込みエラーを値として返し、`main` で「パイプが閉じた」だけなら
//! 成功として扱う。

use std::io::Write;

/// `println!` の代わり。書き込みエラーを `?` で呼び出し元に返す。
macro_rules! outln {
    () => { $crate::out::write_line(format_args!(""))? };
    ($($arg:tt)*) => { $crate::out::write_line(format_args!($($arg)*))? };
}

pub fn write_line(args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_fmt(args)?;
    out.write_all(b"\n")
}

/// このエラーは「読み手が先に閉じた」だけか。
///
/// Windows では ERROR_NO_DATA(232) / ERROR_BROKEN_PIPE(109) が、
/// Unix では EPIPE が `BrokenPipe` に写る。OS ごとの分岐は std がやるので、
/// ここに `#[cfg]` は要らない。
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
    })
}
