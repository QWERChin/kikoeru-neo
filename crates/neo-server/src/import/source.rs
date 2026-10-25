//! Finds the old database and covers from whatever the user points at: the
//! old data directory, its `sqlite/` or `config/` folder, the database file
//! itself, or a Docker volume directory. It only looks at directory entries,
//! the first bytes of candidate files and the whitelisted part of the old
//! `config.json`. Opening the old database here could leave `-wal` or
//! `-shm` files next to it, so that waits for the snapshot.

use std::{
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use serde::Serialize;

use super::error::{ErrorCode, ImportError, refuse};

use super::legacy_config::{self, LegacyConfig, PathMap};

/// Where a path came from, so the user can tell a path they gave from one
/// the import worked out.
/// Serialized as one string: `given`, `default`, `guessed` or
/// `config:<key>`, so every origin has the same JSON type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Named on the command line or picked in a dialog.
    Given,
    /// The old layout's fixed place inside the data directory.
    Default,
    /// Inferred from a neighbouring folder, or from an old path whose
    /// folder had to be swapped for this machine's. Needs the user's
    /// confirmation.
    Guessed,
    /// From this key of the old `config.json`, mapped to this machine.
    Config(&'static str),
}

impl Serialize for Origin {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Config(key) => s.serialize_str(&format!("config:{key}")),
            other => s.serialize_str(&other.to_string()),
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Given => f.write_str("given"),
            Self::Default => f.write_str("default"),
            Self::Guessed => f.write_str("guessed"),
            Self::Config(key) => write!(f, "config.json {key}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Located {
    pub path: PathBuf,
    pub origin: Origin,
}

/// The old data the import reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub db: Located,
    /// Missing covers do not stop an import. Metadata refresh fetches them
    /// again later.
    pub covers: Option<Located>,
    pub config: Option<Located>,
    /// The whitelisted part of `config`.
    pub settings: Option<LegacyConfig>,
}

impl Source {
    /// Paths the user has to confirm before the import uses them.
    pub fn guessed(&self) -> Vec<&Path> {
        [Some(&self.db), self.covers.as_ref()]
            .into_iter()
            .flatten()
            .filter(|l| l.origin == Origin::Guessed)
            .map(|l| l.path.as_path())
            .collect()
    }
}

/// Overrides for the parts of the layout the user names directly.
#[derive(Debug, Clone, Default)]
pub struct Given<'a> {
    pub db: Option<&'a Path>,
    pub covers: Option<&'a Path>,
    pub config: Option<&'a Path>,
    /// Rules for the old paths in `config.json`.
    pub map: Option<&'a PathMap>,
}

const DB: &str = "sqlite/db.sqlite3";
const COVERS: &str = "covers";
/// Docker keeps a named volume's files in `_data` under the volume's
/// directory, which is what `docker volume inspect` prints.
const VOLUME_DATA: &str = "_data";

/// Finds the database and covers from `pick`. Several data directories
/// directly inside `pick` (such as one folder per old version) are an
/// error that lists them, because choosing one is the user's call.
pub fn locate(pick: &Path, given: &Given<'_>) -> Result<Source> {
    let config = match given.config {
        Some(c) => Some(Located {
            path: std::path::absolute(c)?,
            origin: Origin::Given,
        }),
        None => find_config(pick)?,
    };
    let settings = config
        .as_ref()
        .map(|c| LegacyConfig::read(&c.path))
        .transpose()?;
    if let Some(s) = &settings {
        s.check_database_type()?;
    }
    let configured = Configured {
        settings: settings.as_ref(),
        data_dir: config.as_ref().and_then(|c| data_dir_of_config(&c.path)),
        map: given.map,
    };
    let db = match given.db {
        Some(db) => {
            check_db(db)?;
            FoundDb::File(Located {
                path: std::path::absolute(db)?,
                origin: Origin::Given,
            })
        }
        // A picked database file is the user's choice, whatever the old
        // config next to it says.
        None if pick.is_file() => find_db(pick)?,
        None => match configured.db()? {
            Some(db) => FoundDb::File(db),
            None => find_db(pick)?,
        },
    };
    let db = match db {
        FoundDb::File(db) => db,
        FoundDb::Installation(path) => {
            // The discovered installation's config may move its database
            // away from the default file that identified the installation.
            let mut source = locate(&path, given)?;
            source.db.origin = Origin::Guessed;
            if let Some(covers) = &mut source.covers
                && covers.origin == Origin::Default
            {
                covers.origin = Origin::Guessed;
            }
            return Ok(source);
        }
    };
    let covers = match given.covers {
        Some(c) if c.is_dir() => Some(Located {
            path: c.to_owned(),
            origin: Origin::Given,
        }),
        Some(c) => bail!("{} is not a directory", c.display()),
        None => {
            // 0.6.17 writes the default covers folder back as an old
            // absolute path, which maps to the same folder as the default.
            let beside = covers_beside(&db);
            match configured.covers() {
                Some(c) if beside.as_ref().is_none_or(|b| b.path != c.path) => Some(c),
                _ => beside,
            }
        }
    };
    Ok(Source {
        db,
        covers,
        config,
        settings,
    })
}

/// `config.json` in the folders a user is likely to pick: the data
/// directory, its `config/`, or its `sqlite/`.
fn find_config(pick: &Path) -> Result<Option<Located>> {
    if pick.is_file() {
        // A picked database file: look in its data directory.
        let parent = std::path::absolute(pick)?;
        let Some(sqlite) = parent.parent() else {
            return Ok(None);
        };
        return find_config(sqlite);
    }
    let pick = std::path::absolute(pick)?;
    let found = legacy_config::beside(&pick.join(VOLUME_DATA))
        .or_else(|| legacy_config::beside(&pick))
        // Only the old layout's own sqlite/ folder looks one level up, and
        // only into config/. A config.json lying in some parent folder
        // could otherwise send the import to another database.
        .or_else(|| {
            let parent = pick.parent()?;
            let config = parent.join("config").join(legacy_config::FILE);
            (pick.file_name()? == "sqlite" && config.is_file()).then_some(config)
        });
    Ok(found.map(|path| Located {
        path,
        origin: Origin::Default,
    }))
}

/// The data directory a `config.json` belongs to on this machine: the
/// parent of its `config/` folder.
pub fn data_dir_of_config(config: &Path) -> Option<PathBuf> {
    let dir = config.parent()?;
    if dir.file_name()? == "config" {
        dir.parent().map(Path::to_owned)
    } else {
        Some(dir.to_owned())
    }
}

/// The database and covers folders the old config names, mapped onto this
/// machine.
struct Configured<'a> {
    settings: Option<&'a LegacyConfig>,
    data_dir: Option<PathBuf>,
    map: Option<&'a PathMap>,
}

impl Configured<'_> {
    fn map(&self, key: &'static str, old: &str) -> Option<Located> {
        let data_dir = self.data_dir.as_deref()?;
        let empty = PathMap::default();
        let old_data_dir = self.settings?.old_data_dir();
        let mapped = self
            .map
            .unwrap_or(&empty)
            .map(old, data_dir, old_data_dir.as_deref())?;
        Some(Located {
            path: mapped.path,
            origin: if mapped.guessed {
                Origin::Guessed
            } else {
                Origin::Config(key)
            },
        })
    }

    /// Only a database moved away from the default place is looked up,
    /// because 0.6.17 writes even the default folder back as an absolute
    /// path of the old machine.
    fn db(&self) -> Result<Option<Located>> {
        let Some(s) = self.settings else {
            return Ok(None);
        };
        if s.db_use_default_path != Some(false) {
            return Ok(None);
        }
        let Some(dir) = &s.database_folder_dir else {
            return Ok(None);
        };
        let Some(folder) = self.map("databaseFolderDir", dir) else {
            refuse!(
                ErrorCode::MovedDatabase,
                "the old config puts the database in {dir}, which is not on this machine; \
                 pass --map-path {dir}=<where it is now>, or --db"
            );
        };
        let db = Located {
            path: folder.path.join("db.sqlite3"),
            ..folder
        };
        check_db(&db.path)?;
        Ok(Some(db))
    }

    fn covers(&self) -> Option<Located> {
        let s = self.settings?;
        if s.cover_use_default_path == Some(true) {
            // The default folder is in the old data directory, which is
            // not next to a database the config moved elsewhere.
            let covers = self.data_dir.as_ref()?.join(COVERS);
            return covers.is_dir().then_some(Located {
                path: covers,
                origin: Origin::Default,
            });
        }
        let covers = self.map("coverFolderDir", s.cover_folder_dir.as_deref()?)?;
        covers.path.is_dir().then_some(covers)
    }
}

enum FoundDb {
    File(Located),
    Installation(PathBuf),
}

fn find_db(pick: &Path) -> Result<FoundDb> {
    let meta = fs::metadata(pick).map_err(|e| not_found(pick, e))?;
    if meta.is_file() {
        check_db(pick)?;
        // Absolute, so that a bare file name still has a parent folder to
        // find covers next to.
        return Ok(FoundDb::File(Located {
            path: std::path::absolute(pick)?,
            origin: Origin::Given,
        }));
    }
    let volume = pick.join(VOLUME_DATA);
    // A volume may hold the whole data directory or only its sqlite/.
    let pick = if volume.join(DB).is_file() || volume.join("db.sqlite3").is_file() {
        volume
    } else {
        // Absolute, so that the parent of a relative `config` exists.
        std::path::absolute(pick)?
    };
    let candidates = [
        Some((pick.join(DB), Origin::Default)),
        // The `sqlite/` folder itself.
        Some((pick.join("db.sqlite3"), Origin::Default)),
        // `config/` sits next to `sqlite/` in the compose layout and in
        // the Windows release, but nothing ties the two together.
        pick.parent().map(|p| (p.join(DB), Origin::Guessed)),
    ];
    for (path, origin) in candidates.into_iter().flatten() {
        if path.is_file() {
            check_db(&path)?;
            return Ok(FoundDb::File(Located { path, origin }));
        }
    }
    let nested: Vec<PathBuf> = fs::read_dir(&pick)
        .map_err(|e| not_found(&pick, e))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let volume = p.join(VOLUME_DATA);
            p.join(DB).is_file() || volume.join(DB).is_file() || volume.join("db.sqlite3").is_file()
        })
        .collect();
    match nested.as_slice() {
        [] => refuse!(
            ErrorCode::SourceNotFound,
            "found no old kikoeru database in {}; choose the old data directory, \
             its sqlite folder, or the db.sqlite3 file",
            pick.display()
        ),
        [one] => Ok(FoundDb::Installation(one.clone())),
        many => {
            let mut sorted: Vec<_> = many.iter().map(|p| p.display().to_string()).collect();
            sorted.sort();
            refuse!(
                ErrorCode::SeveralSources,
                "{} holds several old data directories; choose one of:\n  {}",
                pick.display(),
                sorted.join("\n  ")
            )
        }
    }
}

/// `covers/` next to the database's `sqlite/` folder. A database that is
/// not in a `sqlite/` folder has no fixed covers place.
fn covers_beside(db: &Located) -> Option<Located> {
    let sqlite = db.path.parent()?;
    if sqlite.file_name()? != "sqlite" {
        return None;
    }
    let covers = sqlite.parent()?.join(COVERS);
    covers.is_dir().then_some(Located {
        path: covers,
        origin: match db.origin {
            Origin::Guessed => Origin::Guessed,
            _ => Origin::Default,
        },
    })
}

/// Checks the SQLite header, so a wrong file is named before anything
/// copies or opens it.
fn check_db(path: &Path) -> Result<()> {
    const HEADER: &[u8; 16] = b"SQLite format 3\0";
    let mut head = [0; 16];
    match fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) if &head == HEADER => Ok(()),
        Err(e) if e.kind() != io::ErrorKind::UnexpectedEof => Err(not_found(path, e)),
        _ => refuse!(
            ErrorCode::NotSqlite,
            "{} is not an SQLite database",
            path.display()
        ),
    }
}

fn not_found(path: &Path, e: io::Error) -> anyhow::Error {
    ImportError::new(
        ErrorCode::SourceNotFound,
        format!("cannot read {}: {e}", path.display()),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &[u8] = b"SQLite format 3\0rest of the page";

    fn data_dir(root: &Path) {
        fs::create_dir_all(root.join("sqlite")).unwrap();
        fs::create_dir_all(root.join("covers")).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join(DB), HEADER).unwrap();
    }

    fn at(path: PathBuf, origin: Origin) -> Located {
        Located { path, origin }
    }

    #[test]
    fn finds_the_database_from_every_folder_of_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        data_dir(&root);
        let db = root.join(DB);
        let covers = Some(at(root.join(COVERS), Origin::Default));
        let none = Given::default();

        let from_root = locate(&root, &none).unwrap();
        assert_eq!(from_root.db, at(db.clone(), Origin::Default));
        assert_eq!(from_root.covers, covers);

        let from_sqlite = locate(&root.join("sqlite"), &none).unwrap();
        assert_eq!(from_sqlite.db, at(db.clone(), Origin::Default));
        assert_eq!(from_sqlite.covers, covers);

        let from_file = locate(&db, &none).unwrap();
        assert_eq!(from_file.db, at(db.clone(), Origin::Given));
        assert_eq!(from_file.covers, covers);

        let from_config = locate(&root.join("config"), &none).unwrap();
        assert_eq!(from_config.db, at(db.clone(), Origin::Guessed));
        assert_eq!(
            from_config.covers,
            Some(at(root.join(COVERS), Origin::Guessed))
        );
        assert_eq!(from_config.guessed().len(), 2);
        assert!(from_root.guessed().is_empty());
    }

    #[test]
    fn looks_inside_a_docker_volume_and_one_level_down() {
        let dir = tempfile::tempdir().unwrap();
        let volume = dir.path().join("kikoeru_sqlite");
        data_dir(&volume.join(VOLUME_DATA));
        let found = locate(&volume, &Given::default()).unwrap();
        assert_eq!(
            found.db,
            at(volume.join(VOLUME_DATA).join(DB), Origin::Default)
        );

        // A volume that held only the old sqlite/ folder.
        let sqlite_volume = dir.path().join("kikoeru_db");
        let data = sqlite_volume.join(VOLUME_DATA);
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("db.sqlite3"), HEADER).unwrap();
        let found = locate(&sqlite_volume, &Given::default()).unwrap();
        assert_eq!(found.db, at(data.join("db.sqlite3"), Origin::Default));
        assert_eq!(found.covers, None);
        let volumes = dir.path().join("volumes");
        fs::create_dir(&volumes).unwrap();
        std::fs::rename(&sqlite_volume, volumes.join("kikoeru_db")).unwrap();
        let found = locate(&volumes, &Given::default()).unwrap();
        assert_eq!(found.db.origin, Origin::Guessed);

        // One data directory inside the chosen folder is a guess.
        let parent = dir.path().join("apps");
        data_dir(&parent.join("kikoeru-v0.6.2"));
        let found = locate(&parent, &Given::default()).unwrap();
        assert_eq!(
            found.db,
            at(parent.join("kikoeru-v0.6.2").join(DB), Origin::Guessed)
        );

        // Two are for the user to choose.
        data_dir(&parent.join("kikoeru-v0.6.14"));
        let err = locate(&parent, &Given::default()).unwrap_err().to_string();
        assert!(err.contains("several old data directories"), "{err}");
        assert!(err.contains("kikoeru-v0.6.14"), "{err}");
        assert!(err.contains("kikoeru-v0.6.2"), "{err}");
    }

    #[test]
    fn a_discovered_installation_respects_path_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("old");
        data_dir(&root);
        let active = root.join("active");
        fs::create_dir(&active).unwrap();
        fs::write(active.join("db.sqlite3"), HEADER).unwrap();
        config(
            &root,
            r#"{"dbUseDefaultPath": false, "databaseFolderDir": "D:\\old-db"}"#,
        );
        let map = PathMap::parse(&[format!(r"D:\old-db={}", active.display())]).unwrap();
        let covers = dir.path().join("my-covers");
        fs::create_dir(&covers).unwrap();
        let mut given = Given {
            covers: Some(&covers),
            map: Some(&map),
            ..Given::default()
        };
        let found = locate(dir.path(), &given).unwrap();
        assert_eq!(found.db, at(active.join("db.sqlite3"), Origin::Guessed));
        assert_eq!(found.covers, Some(at(covers.clone(), Origin::Given)));

        let override_config = dir.path().join("override.json");
        fs::write(&override_config, r#"{"dbUseDefaultPath": true}"#).unwrap();
        given.config = Some(&override_config);
        let found = locate(dir.path(), &given).unwrap();
        assert_eq!(found.db, at(root.join(DB), Origin::Guessed));
        assert_eq!(
            found.config,
            Some(at(override_config.clone(), Origin::Given))
        );
        assert_eq!(found.settings.unwrap().db_use_default_path, Some(true));
        assert_eq!(found.covers, Some(at(covers.clone(), Origin::Given)));

        let db = root.join(DB);
        given.config = None;
        given.db = Some(&db);
        let found = locate(dir.path(), &given).unwrap();
        assert_eq!(found.db, at(db.clone(), Origin::Given));
        assert_eq!(found.covers, Some(at(covers, Origin::Given)));
        assert!(found.guessed().is_empty());
    }

    #[test]
    fn given_paths_win_and_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("old");
        data_dir(&root);
        let elsewhere = dir.path().join("my-covers");
        fs::create_dir(&elsewhere).unwrap();
        let db = dir.path().join("renamed.db");
        fs::copy(root.join(DB), &db).unwrap();

        let found = locate(
            &root,
            &Given {
                db: Some(&db),
                covers: Some(&elsewhere),
                ..Given::default()
            },
        )
        .unwrap();
        assert_eq!(found.db, at(db.clone(), Origin::Given));
        assert_eq!(found.covers, Some(at(elsewhere, Origin::Given)));
        // Without a sqlite/ folder there is no default covers place.
        assert_eq!(locate(&db, &Given::default()).unwrap().covers, None);

        let text = dir.path().join("notes.txt");
        fs::write(&text, "not a database").unwrap();
        let err = locate(&text, &Given::default()).unwrap_err().to_string();
        assert!(err.contains("not an SQLite database"), "{err}");
        let short = dir.path().join("short");
        fs::write(&short, "SQLite").unwrap();
        let err = locate(&short, &Given::default()).unwrap_err().to_string();
        assert!(err.contains("not an SQLite database"), "{err}");
    }

    fn config(root: &Path, json: &str) {
        fs::write(root.join("config/config.json"), json).unwrap();
    }

    #[test]
    fn a_0_6_17_config_with_old_absolute_paths_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        data_dir(&root);
        config(
            &root,
            r#"{"dbUseDefaultPath": true, "databaseFolderDir": "/usr/src/kikoeru/sqlite",
                "coverUseDefaultPath": false, "coverFolderDir": "/usr/src/kikoeru/covers",
                "jwtsecret": "x"}"#,
        );
        let found = locate(&root, &Given::default()).unwrap();
        assert_eq!(found.db, at(root.join(DB), Origin::Default));
        assert_eq!(found.covers, Some(at(root.join(COVERS), Origin::Default)));
        assert_eq!(
            found.config,
            Some(at(root.join("config/config.json"), Origin::Default))
        );
        assert!(found.guessed().is_empty());
        // Picking the database file finds the same config.
        let from_file = locate(&root.join(DB), &Given::default()).unwrap();
        assert_eq!(from_file.config, found.config);
    }

    #[test]
    fn follows_folders_the_old_config_moved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("mycovers")).unwrap();
        let moved = dir.path().join("disk2/db");
        fs::create_dir_all(&moved).unwrap();
        fs::write(moved.join("db.sqlite3"), HEADER).unwrap();
        config(
            &root,
            r#"{"dbUseDefaultPath": false, "databaseFolderDir": "D:\\kikoeru-db",
                "coverFolderDir": "./mycovers"}"#,
        );
        let err = locate(&root, &Given::default()).unwrap_err().to_string();
        assert!(err.contains("--map-path"), "{err}");

        let map = PathMap::parse(&[format!(r"d:\kikoeru-db={}", moved.display())]).unwrap();
        let given = Given {
            map: Some(&map),
            ..Given::default()
        };
        let found = locate(&root, &given).unwrap();
        assert_eq!(
            found.db,
            at(
                moved.join("db.sqlite3"),
                Origin::Config("databaseFolderDir")
            )
        );
        assert_eq!(
            found.covers,
            Some(at(root.join("mycovers"), Origin::Config("coverFolderDir")))
        );
    }

    #[test]
    fn ignores_a_config_json_in_an_unrelated_parent_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        data_dir(&root);
        // Someone else's config.json next to the chosen data directory.
        fs::write(
            dir.path().join("config.json"),
            r#"{"dbUseDefaultPath": false, "databaseFolderDir": "/elsewhere"}"#,
        )
        .unwrap();
        fs::create_dir(dir.path().join("config")).unwrap();
        fs::write(dir.path().join("config/config.json"), "{}").unwrap();
        let found = locate(&root, &Given::default()).unwrap();
        assert_eq!(found.config, None);
        assert_eq!(found.db, at(root.join(DB), Origin::Default));
        // The old layout's own config/ is still found from sqlite/.
        config(&root, "{}");
        let found = locate(&root.join("sqlite"), &Given::default()).unwrap();
        assert_eq!(
            found.config,
            Some(at(root.join("config/config.json"), Origin::Default))
        );
    }

    #[test]
    fn default_covers_stay_in_the_data_directory_when_the_db_moved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join(COVERS)).unwrap();
        let moved = dir.path().join("db");
        fs::create_dir_all(&moved).unwrap();
        fs::write(moved.join("db.sqlite3"), HEADER).unwrap();
        config(
            &root,
            &format!(
                r#"{{"dbUseDefaultPath": false, "databaseFolderDir": "{}",
                    "coverUseDefaultPath": true}}"#,
                moved.display()
            ),
        );
        let found = locate(&root, &Given::default()).unwrap();
        assert_eq!(found.covers, Some(at(root.join(COVERS), Origin::Default)));
    }

    #[test]
    fn a_picked_database_file_wins_over_the_old_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        data_dir(&root);
        config(
            &root,
            r#"{"dbUseDefaultPath": false, "databaseFolderDir": "/gone/db"}"#,
        );
        let found = locate(&root.join(DB), &Given::default()).unwrap();
        assert_eq!(found.db, at(root.join(DB), Origin::Given));
    }

    #[test]
    fn refuses_a_mysql_library() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kikoeru");
        data_dir(&root);
        config(&root, r#"{"sqliteType": "mysql"}"#);
        let err = locate(&root, &Given::default()).unwrap_err().to_string();
        assert!(err.contains("mysql"), "{err}");
    }

    #[test]
    fn says_what_it_looked_for() {
        let dir = tempfile::tempdir().unwrap();
        let err = locate(dir.path(), &Given::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("found no old kikoeru database"), "{err}");
        let err = locate(&dir.path().join("missing"), &Given::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot read"), "{err}");
    }
}
