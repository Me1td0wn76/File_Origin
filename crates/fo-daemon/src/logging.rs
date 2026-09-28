//! デーモンのログ。
//!
//! コンソール窓を消すと `eprintln!` の行き先が無くなる。**先にここを用意してから**
//! `windows_subsystem` を付ける。逆順にすると、動かなくなった理由を誰も追えなくなる。
//!
//! 外部のログクレートは使わない。出力は行指向で単純、必要なのは
//! 「時刻付きで書く」「肥大化させない」「機微情報を出さない」の 3 つだけで、
//! `tracing` 一式を持ち込むほどの要求がない。
//!
//! ## 機微情報の扱い
//!
//! 入手元 URL は閲覧履歴と同等の機微情報（README §14）。**ログファイルにも同じ配慮が要る。**
//! 既定では URL をホストまでに削り、`--verbose` を付けたときだけ全体を出す。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;

/// 1 ファイルの上限。超えたら世代を繰り上げる。
///
/// 監視イベントは頻繁に出るので、上限が無いとディスクを静かに食い潰す。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// 残す世代数（`.1` `.2` …）。現行ファイルを含めない数。
const KEEP: usize = 3;

pub struct Logger {
    inner: Mutex<Inner>,
    path: PathBuf,
    /// 端末にも出すか（`--foreground`）。
    to_stderr: bool,
    /// URL を削らずに出すか（`--verbose`）。
    verbose: bool,
}

struct Inner {
    file: Option<File>,
    written: u64,
}

impl Logger {
    /// ログファイルを開く。開けなくても失敗させない。
    ///
    /// ログが書けないことを理由にデーモンを止めるのは本末転倒。
    /// 記録本体（SQLite）は別で、そちらは開けなければ止まる。
    pub fn open(dir: &Path, to_stderr: bool, verbose: bool) -> Self {
        let path = dir.join("fo-daemon.log");
        let _ = std::fs::create_dir_all(dir);

        let (file, written) = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => {
                let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                (Some(f), size)
            }
            Err(e) => {
                // ここだけは stderr に出す。まだコンソールがあるかもしれない。
                eprintln!("ログを開けません {}: {e}", path.display());
                (None, 0)
            }
        };

        Self {
            inner: Mutex::new(Inner { file, written }),
            path,
            to_stderr,
            verbose,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 1 行書く。
    pub fn log(&self, line: &str) {
        let stamped = format!("{} {line}\n", Local::now().format("%Y-%m-%d %H:%M:%S"));

        if self.to_stderr {
            // 端末側はそのまま出す。時刻はログファイルの都合なので付けない。
            eprintln!("{line}");
        }

        let Ok(mut inner) = self.inner.lock() else {
            return; // ログのために panic を伝播させない
        };
        let Some(file) = inner.file.as_mut() else {
            return;
        };

        if file.write_all(stamped.as_bytes()).is_err() {
            return;
        }
        inner.written += stamped.len() as u64;

        if inner.written >= MAX_BYTES {
            self.rotate(&mut inner);
        }
    }

    /// 世代を繰り上げて新しいファイルにする。
    fn rotate(&self, inner: &mut Inner) {
        inner.file = None; // 先に閉じる。Windows は開いたままだと名前を変えられない

        // 古い順に繰り上げる。KEEP 世代目は捨てる。
        for i in (1..=KEEP).rev() {
            let from = if i == 1 {
                self.path.clone()
            } else {
                self.generation(i - 1)
            };
            let to = self.generation(i);
            if from.exists() {
                let _ = std::fs::rename(&from, &to);
            }
        }

        inner.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
        inner.written = 0;
    }

    fn generation(&self, n: usize) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{n}"));
        self.path.with_file_name(name)
    }

    /// URL をログに出せる形にする。
    ///
    /// 既定はスキームとホストまで。パスとクエリには、どのファイルを取ったか、
    /// どの文書を見ていたかが入る（実データの SharePoint の URL には
    /// 文書 ID が含まれていた）。
    pub fn url(&self, url: Option<&str>) -> String {
        let Some(u) = url else {
            return "(URL なし)".to_string();
        };
        if self.verbose {
            return u.to_string();
        }
        match redact_url(u) {
            Some(short) => short,
            None => "(URL)".to_string(),
        }
    }
}

/// URL をスキームとホストまでに削る。
///
/// `file://` は残りがローカルパスで、そこにもファイル名が出る。
/// ホスト部が空なのでこの関数は `file://` を返し、パスは落ちる。
pub fn redact_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // 認証情報は落とす
    let host = authority.rsplit('@').next().unwrap_or(authority);
    if host.is_empty() {
        Some(format!("{scheme}://"))
    } else {
        Some(format!("{scheme}://{host}/…"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_path_and_query() {
        // 実データの SharePoint の URL にはクエリに文書 ID が入っていた。
        assert_eq!(
            redact_url("https://e.example/sites/x/download.aspx?UniqueId=abc").as_deref(),
            Some("https://e.example/…")
        );
    }

    #[test]
    fn redacts_credentials() {
        assert_eq!(
            redact_url("https://user:pw@e.example/a").as_deref(),
            Some("https://e.example/…")
        );
    }

    #[test]
    fn file_url_loses_its_path() {
        // file:/// はホストが空。展開元の書庫名もファイル名なので残さない。
        assert_eq!(
            redact_url("file:///C:/Users/me/Downloads/秘密の資料.zip").as_deref(),
            Some("file://")
        );
    }

    #[test]
    fn non_hierarchical_urls_are_not_leaked() {
        // `://` が無いものは削りようがないので、呼び出し側が伏せる。
        assert_eq!(redact_url("about:internet"), None);
        assert_eq!(redact_url("mailto:a@b.example"), None);
    }

    #[test]
    fn verbose_keeps_the_whole_url() {
        let dir = std::env::temp_dir().join("fo-log-test-verbose");
        let quiet = Logger::open(&dir, false, false);
        let loud = Logger::open(&dir, false, true);
        let u = "https://e.example/a/b?c=d";

        assert_eq!(quiet.url(Some(u)), "https://e.example/…");
        assert_eq!(loud.url(Some(u)), u);
    }

    #[test]
    fn rotates_when_over_limit() {
        let dir = std::env::temp_dir().join("fo-log-test-rotate");
        let _ = std::fs::remove_dir_all(&dir);
        let log = Logger::open(&dir, false, false);

        // 上限を超えるまで書く。ローテーションが起きれば現行ファイルは小さくなる。
        let line = "x".repeat(4096);
        let mut wrote = 0u64;
        while wrote < MAX_BYTES + 8192 {
            log.log(&line);
            wrote += line.len() as u64 + 24;
        }

        let current = std::fs::metadata(log.path()).map(|m| m.len()).unwrap_or(0);
        assert!(
            current < MAX_BYTES,
            "現行ファイルが上限を超えたまま: {current}"
        );
        assert!(
            log.generation(1).exists(),
            "繰り上げた世代が無い: {:?}",
            log.generation(1)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
