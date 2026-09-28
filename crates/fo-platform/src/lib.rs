//! プラットフォーム抽象化層。
//!
//! **このクレートは workspace で唯一 OS 固有のコードを持つ場所。**
//! `#[cfg(windows)]` / `#[cfg(target_os = "linux")]` と OS 固有クレート
//! （`windows-sys` / `xattr` など）への依存は、ここの外に書かない。
//! 検査は `scripts/arch-guard.sh` と CI が行う。
//!
//! 背景は `docs/adr/0003-platform-abstraction.md` を参照。

use std::fmt;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// この環境ではその機能自体が使えない。呼び出し側はフォールバックする。
    /// 「使えない」と「使えたが結果が空」は区別する必要があるため、専用の値にしてある。
    #[error("この環境では未対応: {0}")]
    Unsupported(&'static str),

    #[error("対象が見つからない: {0}")]
    NotFound(PathBuf),

    #[error("権限が足りない: {0}")]
    PermissionDenied(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// 値型
// ---------------------------------------------------------------------------

/// ボリュームの識別子。
/// Windows は `VolumeSerialNumber`、Linux は `st_dev` を文字列化したもの。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VolumeId(pub String);

/// ボリューム内でファイルを一意に指す鍵。
/// Windows は `FileId128`、Linux は `st_ino`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileKey(pub String);

/// 移動・リネームを跨いで不変なファイル識別子。
///
/// File Origin の同一性判定はこれを主、SHA-256 を従として組み合わせる。
/// ハッシュは「同じ内容か」には答えるが「どこへ行ったか」には答えないため。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StableFileId {
    pub volume: VolumeId,
    pub file: FileKey,
}

impl fmt::Display for StableFileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.volume.0, self.file.0)
    }
}

/// OS が保持していた入手元メタデータ。
///
/// ブラウザ拡張や手動登録は OS の関知するところではないので、ここには現れない。
/// それらは `fo-core` 側の `OriginSource` が扱う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsOrigin {
    pub url: Option<String>,
    pub referrer_url: Option<String>,
    pub source: OsOriginSource,
    /// パースする前の生の値。取りこぼしの調査に使う。
    pub raw: Option<String>,
}

/// OS が入手元を保持している経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsOriginSource {
    /// Windows: NTFS 代替データストリーム `<file>:Zone.Identifier`
    ZoneIdentifier,
    /// Linux: 拡張属性 `user.xdg.origin.url`
    Xattr,
    /// Linux: GVFS メタデータ `metadata::download-uri`（既定 OFF・オプトイン）
    Gvfs,
}

impl OsOriginSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZoneIdentifier => "zone_identifier",
            Self::Xattr => "xattr",
            Self::Gvfs => "gvfs",
        }
    }
}

// ---------------------------------------------------------------------------
// 能力の申告
// ---------------------------------------------------------------------------

/// 機能が「使える / 特権があれば使える / そもそも無い」のどれかを表す。
///
/// 能力差を黙って握りつぶすと、ユーザーは「動いているが何も記録されない」状態に陥る。
/// `fo doctor` がこれを表示して、環境の実力を説明する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// 使える。
    Available,
    /// 仕組みは存在するが、特権が足りず今は使えない。
    NeedsPrivilege { how: &'static str },
    /// ユーザーが明示的に有効化していない（オプトイン待ち）。
    OptIn { how: &'static str },
    /// この OS には該当する仕組みが無い。
    Unavailable { why: &'static str },
}

impl Capability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// 実行環境の実力。`fo doctor` の出力そのもの。
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub platform_name: String,
    /// 安定識別子を取得できるか。
    pub stable_file_id: Capability,
    /// 識別子からパスを逆引きできるか（Linux は原理的に不可）。
    pub reverse_lookup: Capability,
    /// OS が持つ入手元メタデータを読めるか。経路ごとに申告する。
    pub origin_sources: Vec<(OsOriginSource, Capability)>,
    /// アプリ停止中の変更を完全に拾える変更ジャーナルがあるか。
    /// 無くても差分スキャンで動く（ADR-0005）。
    pub change_journal: Capability,
    /// その OS 固有の注意書き。`fo doctor` がそのまま表示する。
    ///
    /// 「API は使えるが実際には期待したものが入っていない」といった、
    /// `Capability` では表せない事情を伝えるための枠。
    /// 例: Linux の xattr は読めるが、主要ブラウザが書かない。
    ///
    /// **これが無いと、OS ごとの分岐が表示側に漏れる。**
    pub advice: Vec<&'static str>,
}

// ---------------------------------------------------------------------------
// trait
// ---------------------------------------------------------------------------

pub trait FileIdentity {
    /// パスから安定識別子を取得する。
    fn stable_id(&self, path: &Path) -> Result<StableFileId>;

    /// 安定識別子から現在のパスを解決する。
    ///
    /// Linux に inode → path の一般的な逆引きは存在しないため、
    /// 呼び出し側は **常に失敗しうる前提** で書く。
    /// 失敗した場合は DB の索引と再スキャンで解決する。
    fn resolve_path(&self, id: &StableFileId) -> Result<Option<PathBuf>>;

    /// 逆引きが OS ネイティブに可能か。
    fn supports_reverse_lookup(&self) -> bool;
}

pub trait OriginMetadata {
    /// OS が保持する入手元メタデータを読む。
    ///
    /// 経路が複数ありうる（Linux は xattr と GVFS の 2 系統）ので `Vec` を返す。
    /// **空の `Vec` は「記録が無かった」であって「読めなかった」ではない。**
    /// 読めるかどうかは `available_sources()` が答える。
    fn read_origin(&self, path: &Path) -> Result<Vec<OsOrigin>>;

    /// この環境で実際に使える読み取り経路。
    fn available_sources(&self) -> Vec<(OsOriginSource, Capability)>;

    /// 自前の記録を OS メタデータとして書き戻す。
    ///
    /// 既定 OFF。ユーザーのファイルを書き換えないのが原則（設計方針 P6）。
    fn write_origin(&self, path: &Path, origin: &OsOrigin) -> Result<()>;
}

pub trait PlatformPaths {
    fn data_dir(&self) -> PathBuf;
    fn config_dir(&self) -> PathBuf;
    fn cache_dir(&self) -> PathBuf;
    fn log_dir(&self) -> PathBuf;
    fn runtime_dir(&self) -> PathBuf;
    /// 既定の監視対象。ユーザーが追加するまでの初期値。
    fn default_download_dirs(&self) -> Vec<PathBuf>;

    /// DB ファイルの位置。
    fn database_path(&self) -> PathBuf {
        self.data_dir().join("file_origin.db")
    }

    /// パスを正規化する。DB に保存する前と、DB を引く前の両方で通す。
    ///
    /// 同じファイルが別の綴りで 2 回記録されるのを防ぐ。
    /// Windows の `std::fs::canonicalize` は `\\?\C:\...` という verbatim 形式を返すが、
    /// ブラウザ拡張や他のツールが渡してくるのは `C:\...` なので、そのままでは一致しない。
    /// 既定実装は std のまま。差がある OS はここを上書きする。
    fn canonical(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }
}

/// すべてを束ねるエントリポイント。アプリは常にこれだけを受け取る。
///
/// TODO(M3): `ChangeJournal`（USN / fanotify）を追加する。既定の差分スキャンで
/// 代替できるため、監視より後回しにしている（ADR-0005）。
/// TODO(M6): `Autostart` を追加する。
pub trait Platform: Send + Sync {
    fn identity(&self) -> &dyn FileIdentity;
    fn origin_meta(&self) -> &dyn OriginMetadata;
    fn paths(&self) -> &dyn PlatformPaths;
    fn ipc(&self) -> &dyn IpcTransport;
    fn host_installer(&self) -> &dyn NativeHostInstaller;

    /// 起動元の端末に標準出力・標準エラーを繋ぐ。繋げたら `true`。
    ///
    /// コンソールを持たないプロセス（Windows の GUI サブシステム）が、
    /// 端末から起動されたときだけ出力を見せるために使う。
    /// **新しいコンソールは作らない。** 窓が勝手に開くのを避けるため。
    ///
    /// Unix は標準出力が最初から繋がっているので何もせず `true` を返す。
    fn attach_parent_console(&self) -> bool;

    /// 新しい監視器を作る。
    ///
    /// `Platform` から借りるのではなく毎回作るのは、監視器が可変状態を持ち、
    /// デーモンのスレッドが専有するため。
    fn new_watcher(&self) -> Result<Box<dyn FsWatcher>>;

    /// 実行環境の実力を診断する。
    fn capabilities(&self) -> Capabilities;
}

// ---------------------------------------------------------------------------
// 実装の切り替え
// ---------------------------------------------------------------------------

pub mod ipc;
pub mod nativehost;
pub mod watcher;

pub use ipc::{IpcListener, IpcName, IpcStream, IpcTransport};
pub use nativehost::{Browser, HostManifest, Installed, NativeHostInstaller};
pub use watcher::{FsEvent, FsWatcher};

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

pub mod mock;

/// **workspace 内で唯一の OS 分岐点。**
///
/// ここ以外で OS を判定したくなったら、その差は trait に表現されていない。
/// trait の設計に戻ること。
pub fn current() -> Box<dyn Platform> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        compile_error!(
            "未対応のプラットフォームです。crates/fo-platform/src/ に実装を追加し、\
             README §7.2 の対応表を更新してください。"
        )
    }
}

/// 環境変数を読み、空文字なら `None` にする。
///
/// 空の `XDG_DATA_HOME` を「設定済み」と誤認すると、パスがルート直下に化ける。
pub(crate) fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
