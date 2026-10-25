//! Copies the old database into a private temporary directory before
//! anything opens it. Opening the user's own file, even read-only, can
//! leave `-wal` or `-shm` files next to it, and a hot journal there would
//! be rolled back in place.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};

use super::error::{ErrorCode, refuse};

use crate::private::PrivateFile;

/// The SQLite side files that can hold committed or half-written changes.
/// The old server never enabled WAL, so after a crash the usual one is the
/// rollback journal, but a database copied from elsewhere may have either.
const SIDE_FILES: [&str; 2] = ["-journal", "-wal"];

/// A private copy of the old database. The directory is removed on drop.
#[derive(Debug)]
pub struct Snapshot {
    _dir: tempfile::TempDir,
    pub db: PathBuf,
}

/// What `take` does with a non-empty journal or WAL next to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journal {
    /// Refuse, so the user stops the old server and lets it close the
    /// database cleanly.
    Refuse,
    /// Let SQLite recover the copy: roll back a hot journal or checkpoint a
    /// WAL. The source stays as it is.
    Recover,
}

/// Identifies one version of a file. A copy whose source changed between
/// the two looks is not a snapshot of any single state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    #[cfg(unix)]
    dev_ino: (u64, u64),
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    fn of(path: &Path) -> io::Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(m) => Ok(Some(Self {
                #[cfg(unix)]
                dev_ino: {
                    use std::os::unix::fs::MetadataExt;
                    (m.dev(), m.ino())
                },
                len: m.len(),
                modified: m.modified().ok(),
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Copies `db` and its non-empty side files, then makes the copy safe to
/// open immutable. `tmp` is where the private directory goes.
pub async fn take(db: &Path, journal: Journal, tmp: &Path) -> Result<Snapshot> {
    // Side files live next to the real file, not next to a symlink to it.
    let real = fs::canonicalize(db).with_context(|| format!("cannot open {}", db.display()))?;
    let dir = tempfile::Builder::new()
        .prefix("kikoeru-import-")
        .tempdir_in(tmp)
        .with_context(|| format!("cannot create a temporary directory in {}", tmp.display()))?;
    let name = real
        .file_name()
        .context("the database path has no file name")?;
    let copy = dir.path().join(name);
    let mut sides = copy_once(&real, &copy)?;
    if sides.is_none() {
        // Written to while copying. One more try covers a single write,
        // but a database that keeps changing has a running server.
        sides = copy_once(&real, &copy)?;
    }
    let Some(sides) = sides else {
        refuse!(
            ErrorCode::SourceChanged,
            "{} changed while it was being copied; stop the old server and try again",
            real.display()
        );
    };
    if !sides.is_empty() {
        let listed: Vec<String> = sides
            .iter()
            .map(|s| with_suffix(&real, s).display().to_string())
            .collect();
        match journal {
            Journal::Refuse => refuse!(
                ErrorCode::UnmergedJournal,
                "{} is not empty, so the database may hold changes that are not in the \
                 main file; stop the old server and let it close the database, or recover \
                 the copy (--recover-journal)",
                listed.join(" and ")
            ),
            Journal::Recover => recover(&copy)
                .await
                .with_context(|| format!("cannot recover the copy of {}", real.display()))?,
        }
    }
    Ok(Snapshot {
        _dir: dir,
        db: copy,
    })
}

/// Copies the database and its non-empty side files. Returns which side
/// files were copied, or `None` when any source file changed meanwhile.
fn copy_once(real: &Path, copy: &Path) -> Result<Option<Vec<&'static str>>> {
    let mut files = vec![(real.to_owned(), copy.to_owned(), "")];
    files.extend(SIDE_FILES.map(|s| (with_suffix(real, s), with_suffix(copy, s), s)));
    let before = stamps(&files)?;
    let mut copied = Vec::new();
    for ((from, to, suffix), stamp) in files.iter().zip(&before) {
        // A side file from the first attempt may be gone from the source
        // by the second, and must not survive in the copy.
        let _ = fs::remove_file(to);
        match stamp {
            // An empty journal is what a clean close leaves behind in some
            // journal modes, and it holds nothing to recover.
            Some(s) if s.len > 0 || suffix.is_empty() => {
                copy_private(from, to)
                    .with_context(|| format!("cannot copy {}", from.display()))?;
                if !suffix.is_empty() {
                    copied.push(*suffix);
                }
            }
            _ => {}
        }
    }
    Ok((stamps(&files)? == before).then_some(copied))
}

/// Copies into a new 0600 file. `fs::copy` would carry over the source's
/// mode, and a read-only source would leave a copy SQLite cannot recover.
fn copy_private(from: &Path, to: &Path) -> io::Result<()> {
    let mut src = fs::File::open(from)?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(to)?;
    io::copy(&mut src, &mut dst)?;
    Ok(())
}

fn stamps(files: &[(PathBuf, PathBuf, &str)]) -> Result<Vec<Option<Stamp>>> {
    files
        .iter()
        .map(|(from, _, _)| {
            Stamp::of(from).with_context(|| format!("cannot read {}", from.display()))
        })
        .collect()
}

/// Opens the copy read-write once, so SQLite rolls back a hot journal or
/// checkpoints a WAL into the main file. `Legacy::load` opens databases
/// immutable, which ignores side files, so this has to happen first.
async fn recover(copy: &Path) -> Result<()> {
    let mut conn = SqliteConnectOptions::new().filename(copy).connect().await?;
    // Reading the schema is what makes SQLite look at the journal.
    sqlx::query("SELECT COUNT(*) FROM sqlite_master")
        .execute(&mut conn)
        .await?;
    // A WAL is folded back only by a checkpoint and switching it off.
    sqlx::query("PRAGMA journal_mode = DELETE")
        .execute(&mut conn)
        .await?;
    sqlx::Connection::close(conn).await?;
    // A journal SQLite did not roll back was not hot, for example one whose
    // header was never synced before a crash, and SQLite leaves it in
    // place. It holds nothing, and immutable reads would refuse it.
    let journal = with_suffix(copy, "-journal");
    match fs::remove_file(&journal) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot remove {}", journal.display())),
    }
    let wal = with_suffix(copy, "-wal");
    if fs::metadata(&wal).is_ok_and(|m| m.len() > 0) {
        bail!("SQLite left {} after recovery", wal.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection, SqliteConnection};

    async fn db_with_rows(path: &Path, rows: i64) {
        let mut conn = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE t (x INTEGER)")
            .execute(&mut conn)
            .await
            .unwrap();
        for x in 0..rows {
            sqlx::query("INSERT INTO t VALUES (?)")
                .bind(x)
                .execute(&mut conn)
                .await
                .unwrap();
        }
        conn.close().await.unwrap();
    }

    async fn rows(path: &Path) -> i64 {
        let mut conn = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .immutable(true)
            .connect()
            .await
            .unwrap();
        let n = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        n
    }

    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    /// A database with a hot journal, as a crash in the middle of a write
    /// leaves it: the main file holds the half-written change, and the
    /// journal holds the pages to roll back to 3 rows.
    async fn crashed(dir: &Path) -> PathBuf {
        let db = dir.join("db.sqlite3");
        db_with_rows(&db, 3).await;
        let mut conn: SqliteConnection = SqliteConnectOptions::new()
            .filename(&db)
            .pragma("journal_mode", "DELETE")
            .pragma("cache_size", "1")
            .connect()
            .await
            .unwrap();
        sqlx::query("BEGIN").execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO t SELECT x + 100 FROM t")
            .execute(&mut conn)
            .await
            .unwrap();
        // A spilled cache writes changed pages to the main file before the
        // commit, which is the state a crash can leave.
        for _ in 0..200 {
            sqlx::query("INSERT INTO t VALUES (randomblob(4000))")
                .execute(&mut conn)
                .await
                .unwrap();
        }
        let journal = with_suffix(&db, "-journal");
        assert!(journal.exists(), "the open transaction keeps a journal");
        // Copy the files as they are mid-transaction, then let the real
        // connection roll back.
        let crashed = dir.join("crashed");
        fs::create_dir(&crashed).unwrap();
        fs::copy(&db, crashed.join("db.sqlite3")).unwrap();
        fs::copy(&journal, crashed.join("db.sqlite3-journal")).unwrap();
        sqlx::query("ROLLBACK").execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();
        crashed.join("db.sqlite3")
    }

    #[tokio::test]
    async fn copies_a_clean_database_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let db = src.join("db.sqlite3");
        db_with_rows(&db, 3).await;
        // An empty journal is not a reason to refuse.
        fs::write(with_suffix(&db, "-journal"), "").unwrap();
        let before = files(&src);

        let snap = take(&db, Journal::Refuse, dir.path()).await.unwrap();
        assert_eq!(rows(&snap.db).await, 3);
        assert_eq!(files(&src), before);
        let tmp = snap.db.parent().unwrap().to_owned();
        assert!(tmp.starts_with(dir.path()));
        drop(snap);
        assert!(!tmp.exists(), "the copy goes away with the snapshot");
    }

    #[tokio::test]
    async fn refuses_or_recovers_a_hot_journal_on_the_copy_only() {
        let dir = tempfile::tempdir().unwrap();
        let db = crashed(dir.path()).await;
        let src = db.parent().unwrap();
        let before = files(src);

        let err = take(&db, Journal::Refuse, dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("--recover-journal"), "{err}");
        assert!(err.contains("db.sqlite3-journal"), "{err}");

        let snap = take(&db, Journal::Recover, dir.path()).await.unwrap();
        assert_eq!(rows(&snap.db).await, 3, "rolled back to the last commit");
        assert!(!with_suffix(&snap.db, "-journal").exists());
        assert_eq!(files(src), before, "the source keeps its journal");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovers_a_read_only_source() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let db = crashed(dir.path()).await;
        let src = db.parent().unwrap();
        for f in fs::read_dir(src).unwrap() {
            fs::set_permissions(f.unwrap().path(), fs::Permissions::from_mode(0o444)).unwrap();
        }
        // The copy is private and writable whatever the source's mode is.
        let snap = take(&db, Journal::Recover, dir.path()).await.unwrap();
        assert_eq!(rows(&snap.db).await, 3);
        let mode = fs::metadata(&snap.db).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn recovers_past_a_journal_that_is_not_hot() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let db = src.join("db.sqlite3");
        db_with_rows(&db, 3).await;
        // A crash before the journal header was synced leaves zeros, which
        // SQLite ignores without deleting the file.
        fs::write(with_suffix(&db, "-journal"), [0u8; 512]).unwrap();
        assert!(take(&db, Journal::Refuse, dir.path()).await.is_err());
        let snap = take(&db, Journal::Recover, dir.path()).await.unwrap();
        assert_eq!(rows(&snap.db).await, 3);
        assert!(with_suffix(&db, "-journal").exists(), "the source keeps it");
    }

    #[tokio::test]
    async fn recovers_a_wal() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let db = src.join("db.sqlite3");
        db_with_rows(&db, 1).await;
        let mut conn = SqliteConnectOptions::new()
            .filename(&db)
            .pragma("journal_mode", "WAL")
            .pragma("wal_autocheckpoint", "0")
            .connect()
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (2), (3)")
            .execute(&mut conn)
            .await
            .unwrap();
        // The commit is only in the WAL while this connection is open.
        let snap = take(&db, Journal::Recover, dir.path()).await.unwrap();
        assert_eq!(rows(&snap.db).await, 3);
        conn.close().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn follows_a_symlink_to_the_real_file_and_its_journal() {
        let dir = tempfile::tempdir().unwrap();
        let db = crashed(dir.path()).await;
        let link = dir.path().join("link.sqlite3");
        std::os::unix::fs::symlink(&db, &link).unwrap();
        let err = take(&link, Journal::Refuse, dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("crashed/db.sqlite3-journal"), "{err}");
    }
}
