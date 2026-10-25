//! What the desktop shell's import commands carry ("外壳给页面的命令" in
//! book/src/desktop.md). The shell builds them from the import's own
//! report, keeping only what the settings page shows.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::RootStatus;

/// Why an import stopped, as listed in book/src/contract.md, "导入的错误码".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ImportErrorCode {
    /// Nothing that looks like an old kikoeru install, or it cannot be read.
    SourceNotFound,
    /// The chosen or found file is not an SQLite database.
    NotSqlite,
    /// The chosen folder holds several old installs.
    SeveralSources,
    /// Some paths were inferred and need the user's confirmation.
    GuessedPaths,
    /// A non-empty journal or WAL sits next to the old database.
    UnmergedJournal,
    /// The old database changed while it was being copied.
    SourceChanged,
    /// The database lacks the tables or columns of a kikoeru library.
    NotKikoeru,
    /// The library comes from a version before 0.6.14.
    TooOld,
    /// The old server stored its library in MySQL.
    Mysql,
    /// The old config.json exists but cannot be read.
    ConfigUnreadable,
    /// The old config moved the database to a folder not on this machine.
    MovedDatabase,
    /// The new data directory is not empty.
    TargetNotEmpty,
    /// The new data directory overlaps the old data.
    TargetOverlaps,
    /// `--merge` names a folder without a library.
    NoLibrary,
    /// The dry run found reasons the import would refuse.
    WouldRefuse,
    /// SIGINT or SIGTERM stopped the import, which then cleaned up.
    Cancelled,
    /// Anything else, such as a disk or database error.
    Other,
}

/// What a failed import command rejects with. The page words its message
/// from `code` and shows `message`, the full reason, underneath.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportFailure {
    pub code: ImportErrorCode,
    pub message: String,
}

/// A path the import found, and how: `given`, `default`, `guessed` or
/// `config:<key>`. A `guessed` path needs the user's confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportPath {
    pub path: String,
    pub origin: String,
}

/// The old data the import reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportSource {
    pub db: ImportPath,
    pub covers: Option<ImportPath>,
    pub config: Option<ImportPath>,
    /// The version the old `config.json` names, such as `0.6.17`.
    pub version: Option<String>,
}

/// The answer of `import_discover`, `null` when the dialog was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportDiscovered {
    /// The folder picked in the dialog, passed back to the other commands.
    pub pick: String,
    pub source: ImportSource,
}

/// Rows read from the old library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportCounts {
    pub works: u32,
    pub users: u32,
    pub favorites: u32,
    pub reviews: u32,
    pub progress: u32,
    /// Works with a favorite, review or position of any old user.
    pub works_with_data: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportUser {
    pub name: String,
    pub is_admin: bool,
}

/// A media library from the old `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportOldRoot {
    pub name: String,
    /// `local`, `openlist`, or another type the old server knew.
    pub kind: String,
    /// The folder as the old server saw it.
    pub old_path: Option<String>,
    /// Where that folder is on this machine, when the import found it.
    pub path: Option<ImportPath>,
    /// A local folder that was found, so `add_roots` may name it.
    pub addable: bool,
    /// For a found local folder with works: how many of the sampled works
    /// it holds. Under half suggests the wrong folder.
    pub sample: Option<ImportSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportSample {
    pub checked: u32,
    pub found: u32,
}

/// A row that will be skipped, changed or left unmatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportIssue {
    pub at: String,
    pub reason: String,
}

/// The answer of `import_preview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportPreview {
    pub source: ImportSource,
    pub counts: ImportCounts,
    pub users: Vec<ImportUser>,
    pub old_roots: Vec<ImportOldRoot>,
    pub issues: Vec<ImportIssue>,
    /// Why the import would refuse, one reason per line. Not empty means
    /// `import_run` fails with `would_refuse`.
    pub refusal: Option<Vec<String>>,
}

/// The answer of `import_run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ImportResult {
    pub works: u32,
    pub favorites: u32,
    pub reviews: u32,
    pub progress: u32,
    /// Positions kept because the local one was newer.
    pub progress_kept: u32,
    /// Old works with data that the local library has not scanned yet.
    pub waiting: u32,
    /// Old users that got their own local account.
    pub users_created: Vec<String>,
    /// Libraries added through the roots API, not scanned yet.
    pub roots_added: Vec<RootStatus>,
}
