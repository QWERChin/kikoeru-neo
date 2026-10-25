//! The data directory layout from book/src/config.md and the JWT signing key
//! stored in it.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::private::PrivateFile;

const KEY_BYTES: usize = 32;

/// The SQLite database's name in the data directory.
pub const DATABASE_FILE: &str = "neo.sqlite3";

#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// Creates the directory tree and an empty database file if needed, and
    /// refuses directories or files that other users could write to or, for
    /// secrets, read. New directories are 0700 and the database file is
    /// 0600. SQLite gives its -wal and -shm files the database's mode.
    pub fn prepare(root: &Path) -> Result<Self> {
        Self::prepare_inner(root, true)
    }

    /// [`prepare`](Self::prepare) for a library whose database is at
    /// `location`. The SQLite file is created only when the database is
    /// SQLite, so a PostgreSQL library has no empty file that a backup or
    /// a merge could take for the library.
    pub fn prepare_for(root: &Path, location: &neo_db::Location) -> Result<Self> {
        Self::prepare_inner(root, matches!(location, neo_db::Location::Sqlite(_)))
    }

    fn prepare_inner(root: &Path, create_sqlite: bool) -> Result<Self> {
        let dir = Self {
            root: root.to_owned(),
        };
        let private_dir = |path: &Path| {
            crate::private::dir()
                .recursive(true)
                .create(path)
                .with_context(|| format!("cannot create {}", path.display()))
        };
        private_dir(root)?;
        let resolved_root = root
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", root.display()))?;
        check_mode(&resolved_root, 0o022, "writable by group or others")?;
        let secrets = root.join("secrets");
        private_dir(&secrets)?;
        check_mode(&secrets, 0o077, "accessible by group or others (chmod 700)")?;
        for sub in [
            "covers",
            "lyrics",
            "cache/transcode",
            "cache/thumbs",
            "cache/tmp",
        ] {
            private_dir(&root.join(sub))?;
        }
        if create_sqlite {
            let db = dir.database();
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .private()
                .open(&db)
                .with_context(|| format!("cannot create {}", db.display()))?;
            check_mode(&db, 0o022, "writable by group or others")?;
        }
        Ok(dir)
    }

    /// A data directory that must already exist. Nothing is created, so a
    /// mistyped path is an error instead of a new, empty library.
    pub fn existing(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!("data directory {} does not exist", root.display());
        }
        Ok(Self {
            root: root.to_owned(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> PathBuf {
        self.root.join(DATABASE_FILE)
    }

    pub fn jwt_key_path(&self) -> PathBuf {
        self.root.join("secrets/jwt.key")
    }

    /// Taken by a server before it touches anything and held while it runs.
    /// A second server on the same directory stops here, before it clears
    /// cache/tmp under the first one's jobs or marks its running tasks
    /// interrupted.
    pub fn lock_instance(&self) -> Result<fs::File> {
        let path = self.root.join("serve.lock");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .private()
            .open(&path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(fs::TryLockError::WouldBlock) => bail!(
                "another kikoeru-neo server is using {}",
                self.root.display()
            ),
            Err(fs::TryLockError::Error(e)) => {
                Err(e).with_context(|| format!("cannot lock {}", path.display()))
            }
        }
    }

    /// Removes leftovers of generation jobs that were running when the
    /// process last stopped. They are never complete.
    pub fn clear_tmp(&self) -> Result<()> {
        let tmp = self.root.join("cache/tmp");
        for entry in fs::read_dir(&tmp).with_context(|| format!("cannot read {}", tmp.display()))? {
            let path = entry?.path();
            let removed = if path.is_dir() && !path.is_symlink() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            removed.with_context(|| format!("cannot remove {}", path.display()))?;
        }
        Ok(())
    }

    /// Loads the signing key, generating it on first start. The key never
    /// goes into the config file, which may live in the world-readable Nix
    /// store.
    pub fn load_or_create_jwt_key(&self) -> Result<Vec<u8>> {
        let path = self.jwt_key_path();
        // symlink_metadata, not metadata: a dangling symlink must count as an
        // existing entry, otherwise publishing below fails with AlreadyExists
        // every time.
        match fs::symlink_metadata(&path) {
            Ok(_) => read_key(&path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut key = vec![0u8; KEY_BYTES];
                getrandom::fill(&mut key)
                    .map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
                if publish_secret(&path, &key)? {
                    tracing::info!(path = %path.display(), "generated a new JWT signing key");
                    Ok(key)
                } else {
                    // Another process published its key first. Use that one,
                    // so both sign with the key that survives a restart.
                    read_key(&path)
                }
            }
            Err(e) => Err(e).with_context(|| format!("cannot stat {}", path.display())),
        }
    }
}

/// Reads an existing key. The key must be a regular file, so a symlink
/// (dangling or not) is refused rather than followed.
fn read_key(path: &Path) -> Result<Vec<u8>> {
    let meta =
        fs::symlink_metadata(path).with_context(|| format!("cannot stat {}", path.display()))?;
    if !meta.file_type().is_file() {
        bail!(
            "{} is not a regular file; remove it to generate a new key",
            path.display()
        );
    }
    check_mode(path, 0o077, "accessible by group or others (chmod 600)")?;
    let key = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    if key.len() < KEY_BYTES {
        bail!(
            "{} holds {} bytes, expected at least {KEY_BYTES}; delete it to generate a new key",
            path.display(),
            key.len()
        );
    }
    Ok(key)
}

/// Fails if `path` has any of the `forbidden` permission bits.
/// Windows has no permission bits, so there the check passes.
fn check_mode(path: &Path, forbidden: u32, problem: &str) -> Result<()> {
    let Some(mode) =
        crate::private::mode(path).with_context(|| format!("cannot stat {}", path.display()))?
    else {
        return Ok(());
    };
    if mode & forbidden != 0 {
        bail!("{} has mode {mode:o}, which is {problem}", path.display());
    }
    Ok(())
}

/// Writes the key to a temporary file unique to this call, then publishes it
/// with `hard_link`, which fails instead of overwriting when `path` already
/// exists. A crash never leaves a truncated key, two processes starting at
/// once never replace each other's key, and the temporary file is created
/// exclusively with mode 0600 so the key is never readable by others.
/// Returns false if another process published a key first.
fn publish_secret(path: &Path, bytes: &[u8]) -> Result<bool> {
    let mut suffix = [0u8; 8];
    getrandom::fill(&mut suffix).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    let tmp = path.with_extension(format!("tmp-{}", u64::from_le_bytes(suffix)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(&tmp)
        .with_context(|| format!("cannot create {}", tmp.display()))?;
    let published = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::hard_link(&tmp, path));
    let _ = fs::remove_file(&tmp);
    match published {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| format!("cannot create {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn prepare_creates_layout_and_key_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(&tmp.path().join("data")).unwrap();
        for sub in [
            "secrets",
            "covers",
            "lyrics",
            "cache/transcode",
            "cache/thumbs",
            "cache/tmp",
        ] {
            assert!(dir.root().join(sub).is_dir(), "{sub}");
        }
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.root()), 0o700);
        assert_eq!(mode(&dir.root().join("secrets")), 0o700);
        assert_eq!(mode(&dir.database()), 0o600);
        let key = dir.load_or_create_jwt_key().unwrap();
        assert_eq!(key.len(), KEY_BYTES);
        let mode = fs::metadata(dir.jwt_key_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(dir.load_or_create_jwt_key().unwrap(), key);
    }

    #[test]
    fn symlinked_data_directory_checks_target_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("data");
        DataDir::prepare(&target).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        DataDir::prepare(&link).unwrap();

        fs::set_permissions(&target, fs::Permissions::from_mode(0o770)).unwrap();
        let err = DataDir::prepare(&link).unwrap_err().to_string();
        assert!(err.contains("mode 770"), "{err}");
        assert!(err.contains("writable by group or others"), "{err}");
    }

    #[test]
    fn readable_key_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        dir.load_or_create_jwt_key().unwrap();
        fs::set_permissions(dir.jwt_key_path(), fs::Permissions::from_mode(0o644)).unwrap();
        let err = dir.load_or_create_jwt_key().unwrap_err().to_string();
        assert!(err.contains("mode 644"), "{err}");
    }

    #[test]
    fn unsafe_directory_modes_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        let set = |p: &Path, m: u32| fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
        let secrets = dir.root().join("secrets");
        set(&secrets, 0o750);
        assert!(DataDir::prepare(tmp.path()).is_err());
        set(&secrets, 0o700);
        set(&dir.database(), 0o666);
        assert!(DataDir::prepare(tmp.path()).is_err());
        set(&dir.database(), 0o644);
        set(dir.root(), 0o777);
        assert!(DataDir::prepare(tmp.path()).is_err());
        // A read-only data directory, as systemd's StateDirectory makes it,
        // is fine.
        set(dir.root(), 0o755);
        DataDir::prepare(tmp.path()).unwrap();
    }

    #[test]
    fn stale_temporary_key_is_not_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        let stale = dir.jwt_key_path().with_extension("tmp");
        fs::write(&stale, b"old").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
        let key = dir.load_or_create_jwt_key().unwrap();
        let mode = fs::metadata(dir.jwt_key_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_ne!(key, b"old");
    }

    #[test]
    fn short_key_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        assert!(publish_secret(&dir.jwt_key_path(), b"short").unwrap());
        assert!(dir.load_or_create_jwt_key().is_err());
    }

    #[test]
    fn symlinked_key_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("nowhere"), dir.jwt_key_path()).unwrap();
        assert!(dir.load_or_create_jwt_key().is_err());
        let target = tmp.path().join("real-key");
        fs::write(&target, [7; KEY_BYTES]).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(dir.jwt_key_path()).unwrap();
        std::os::unix::fs::symlink(&target, dir.jwt_key_path()).unwrap();
        assert!(dir.load_or_create_jwt_key().is_err());
    }

    #[test]
    fn publish_never_overwrites_an_existing_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        let path = dir.jwt_key_path();
        assert!(publish_secret(&path, &[1; KEY_BYTES]).unwrap());
        assert!(!publish_secret(&path, &[2; KEY_BYTES]).unwrap());
        assert_eq!(fs::read(&path).unwrap(), [1; KEY_BYTES]);
    }

    #[test]
    fn concurrent_first_starts_agree_on_the_key() {
        for _ in 0..20 {
            let tmp = tempfile::tempdir().unwrap();
            let dir = DataDir::prepare(tmp.path()).unwrap();
            let barrier = std::sync::Barrier::new(8);
            let keys: Vec<Vec<u8>> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..8)
                    .map(|_| {
                        s.spawn(|| {
                            barrier.wait();
                            dir.load_or_create_jwt_key().unwrap()
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let on_disk = fs::read(dir.jwt_key_path()).unwrap();
            assert!(keys.iter().all(|k| *k == on_disk));
            let leftovers = fs::read_dir(dir.root().join("secrets")).unwrap().count();
            assert_eq!(leftovers, 1, "only jwt.key should remain");
        }
    }

    #[test]
    fn clear_tmp_empties_only_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::prepare(tmp.path()).unwrap();
        fs::write(dir.root().join("cache/tmp/partial"), b"x").unwrap();
        fs::create_dir(dir.root().join("cache/tmp/job")).unwrap();
        fs::write(dir.root().join("cache/transcode/done"), b"x").unwrap();
        dir.clear_tmp().unwrap();
        assert_eq!(
            fs::read_dir(dir.root().join("cache/tmp")).unwrap().count(),
            0
        );
        assert!(dir.root().join("cache/transcode/done").exists());
    }
}
