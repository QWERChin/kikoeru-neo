//! `kikoeru-neo backup`: a consistent copy of the service state, taken
//! while the server may be running (book/src/config.md "备份").

use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use sqlx::{ConnectOptions, Connection, sqlite::SqliteConnectOptions};

use neo_db::Location;

use crate::datadir::DataDir;

/// Directories and files of the data directory that hold state. `cache/`
/// is rebuilt on demand and left out.
const STATE: [&str; 4] = ["secrets", "covers", "lyrics", "import-report.json"];

/// How long the copy waits for a writer to finish before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Copied {
    pub files: u64,
    pub bytes: u64,
    /// Symlinks are not followed, so a link cannot pull a file from outside
    /// the data directory into the backup.
    pub skipped_links: u64,
}

/// Writes the backup into `dest`, which must not exist yet. Its parent must.
///
/// A SQLite database is copied with `VACUUM INTO`, which reads one snapshot
/// in a single transaction. Copying `neo.sqlite3` by itself would miss what
/// is still in the `-wal` file. A PostgreSQL database is copied into
/// `neo.sqlite3` too, from one snapshot, so every backup has the same layout
/// and needs no `pg_dump` of a matching version to restore.
pub async fn run(data_dir: &DataDir, database: &Location, dest: &Path) -> Result<Copied> {
    if let Location::Sqlite(db) = database
        && !db.is_file()
    {
        bail!("{} holds no database", data_dir.root().display());
    }
    refuse_inside(data_dir.root(), dest)?;
    crate::private::dir()
        .create(dest)
        .with_context(|| format!("cannot create {} (it must not exist yet)", dest.display()))?;
    // Held over the database copy and the covers copy, so a metadata
    // refresh cannot replace a cover between the two.
    let covers = data_dir.root().join("covers");
    let _covers_lock = if covers.is_dir() {
        Some(
            tokio::task::spawn_blocking(move || neo_scanner::metadata::lock_covers(&covers))
                .await?
                .context("cannot lock covers/ against cover updates")?,
        )
    } else {
        None
    };
    // Held over the database copy and the secrets copy, so a root changed
    // in between cannot leave its settings and password out of step.
    let secrets_root = data_dir.root().to_owned();
    let _secrets_lock =
        tokio::task::spawn_blocking(move || crate::roots::lock_secrets(&secrets_root)).await??;
    let target = dest.join(crate::datadir::DATABASE_FILE);
    match database {
        Location::Sqlite(db) => vacuum_into(db, &target).await?,
        #[cfg(feature = "postgres")]
        Location::Postgres(url) => {
            use neo_db::copy::{Endpoint, copy};
            copy(
                &Endpoint::Postgres(url.clone()),
                &Endpoint::Sqlite(target.clone()),
            )
            .await
            .with_context(|| format!("cannot copy the database to {}", target.display()))?;
        }
    }
    crate::private::restrict_file(&target)?;

    let mut copied = Copied {
        files: 1,
        bytes: fs::metadata(&target)?.len(),
        skipped_links: 0,
    };
    for name in STATE {
        let from = data_dir.root().join(name);
        if fs::symlink_metadata(&from).is_ok() {
            copy_tree(&from, &dest.join(name), &mut copied)?;
        }
    }
    Ok(copied)
}

async fn vacuum_into(db: &Path, target: &Path) -> Result<()> {
    let target_str = target
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", target.display()))?;
    // Not `Db::open`: a backup must not migrate the database it copies.
    let mut conn = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(false)
        .busy_timeout(BUSY_TIMEOUT)
        .connect()
        .await
        .with_context(|| format!("cannot open {}", db.display()))?;
    let vacuum = sqlx::query("VACUUM INTO ?")
        .bind(target_str)
        .execute(&mut conn)
        .await;
    conn.close().await.ok();
    vacuum.with_context(|| format!("cannot copy the database to {}", target.display()))?;
    Ok(())
}

/// A backup inside the data directory would copy itself: under `covers/`
/// the recursive copy keeps finding its own output. Both paths are resolved
/// first, so a symlinked parent cannot hide the overlap.
fn refuse_inside(root: &Path, dest: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
        bail!("{} is not a directory that can be created", dest.display());
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let resolved = parent
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", parent.display()))?
        .join(name);
    if resolved.starts_with(&root) {
        bail!(
            "{} is inside the data directory {}; write the backup somewhere else",
            dest.display(),
            root.display()
        );
    }
    Ok(())
}

/// Copies files and directories under `from`. Everything written is private
/// to the owner, as in the data directory itself.
fn copy_tree(from: &Path, to: &Path, copied: &mut Copied) -> Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        crate::private::dir()
            .create(to)
            .with_context(|| format!("cannot create {}", to.display()))?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()), copied)?;
        }
    } else if meta.is_file() {
        let bytes =
            fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
        crate::private::restrict_file(to)?;
        copied.files += 1;
        copied.bytes += bytes;
    } else {
        copied.skipped_links += 1;
    }
    Ok(())
}
