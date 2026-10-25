//! The steps every way of importing shares: find the old data, copy the
//! database, and read it. `import-legacy` uses it now. The planned
//! `migrate` command and the desktop shell's import are meant to use the
//! same steps (book/src/migration.md, "找旧数据").

use std::path::Path;

use anyhow::Result;

use super::error::{ErrorCode, refuse};

use super::{
    legacy::Legacy,
    snapshot::{self, Journal},
    source::{self, Given, Source},
};

/// An old installation, read into memory. The private copy of its database
/// is already gone, because everything the import needs is in `legacy`.
#[derive(Debug)]
pub struct Session {
    pub source: Source,
    pub legacy: Legacy,
}

#[derive(Debug, Clone)]
pub struct Options<'a> {
    pub given: Given<'a>,
    /// The user confirmed the paths marked as guessed.
    pub accept_guessed: bool,
    pub journal: Journal,
    /// Where the private copy of the database goes.
    pub tmp: &'a Path,
}

impl Session {
    pub async fn open(pick: &Path, options: &Options<'_>) -> Result<Self> {
        let source = source::locate(pick, &options.given)?;
        let guessed = source.guessed();
        if !guessed.is_empty() && !options.accept_guessed {
            let listed: Vec<String> = guessed.iter().map(|p| p.display().to_string()).collect();
            refuse!(
                ErrorCode::GuessedPaths,
                "these paths were inferred from a neighbouring folder:\n  {}\n\
                 check them and pass --accept-guessed, or choose the old data directory itself",
                listed.join("\n  ")
            );
        }
        let snapshot = snapshot::take(&source.db.path, options.journal, options.tmp).await?;
        let legacy = Legacy::load_as(&snapshot.db, &source.db.path).await?;
        Ok(Self { source, legacy })
    }

    pub fn covers(&self) -> Option<&Path> {
        self.source.covers.as_ref().map(|c| c.path.as_path())
    }
}
