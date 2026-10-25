//! Reads the old `config.json`, keeping only the fields the import needs:
//! where the database and covers are, the database type, whether login was
//! on, and the media library folders. The file also holds the old JWT and
//! MD5 secrets and OpenList passwords. Those keys are not fields of any
//! struct here, so serde skips them without keeping their values.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de::IgnoredAny};

use super::error::{ErrorCode, ImportError, refuse};

pub const FILE: &str = "config.json";

/// The whitelisted part of the old config. Every field is optional,
/// because older versions wrote fewer keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyConfig {
    /// The config format version the old server wrote, such as `0.6.17`.
    pub version: Option<String>,
    /// `sqlite3`, or `mysql` for a library without an SQLite file.
    pub sqlite_type: Option<String>,
    pub db_use_default_path: Option<bool>,
    pub database_folder_dir: Option<String>,
    pub cover_use_default_path: Option<bool>,
    pub cover_folder_dir: Option<String>,
    pub auth: Option<bool>,
    #[serde(default, deserialize_with = "valid_roots")]
    pub root_folders: Vec<RootFolder>,
}

/// An old media library. `username` names the OpenList account and is not
/// a secret. The password is never read, so it has to be entered again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFolder {
    pub name: String,
    /// `local` when missing, as the old server assumed.
    #[serde(rename = "type", default = "local")]
    pub kind: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub base_path: Option<String>,
    pub username: Option<String>,
}

fn local() -> String {
    "local".to_owned()
}

/// Skips entries the old server skipped too, such as one without a name,
/// instead of failing the whole file.
fn valid_roots<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RootFolder>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Root(RootFolder),
        Other(IgnoredAny),
    }
    let entries = Option::<Vec<Entry>>::deserialize(d)?.unwrap_or_default();
    Ok(entries
        .into_iter()
        .filter_map(|e| match e {
            Entry::Root(r) if !r.name.is_empty() => Some(r),
            _ => None,
        })
        .collect())
}

impl LegacyConfig {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|e| {
            ImportError::new(
                ErrorCode::ConfigUnreadable,
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
        serde_json::from_slice(&bytes).map_err(|e| {
            ImportError::new(
                ErrorCode::ConfigUnreadable,
                format!("{} is not a kikoeru config.json: {e}", path.display()),
            )
            .into()
        })
    }

    /// Refuses a library the import cannot read at all.
    pub fn check_database_type(&self) -> Result<()> {
        match self.sqlite_type.as_deref() {
            None | Some("sqlite3" | "sqlite") => Ok(()),
            Some(other) => refuse!(
                ErrorCode::Mysql,
                "the old server stored its library in {other}, not in an SQLite file; \
                 the import only reads SQLite"
            ),
        }
    }

    /// The old data directory as the old server saw it. 0.6.17 writes the
    /// default database and covers folders back as absolute paths under it.
    pub fn old_data_dir(&self) -> Option<String> {
        let under = |dir: Option<&String>, name: &str| {
            let dir = dir?;
            let style = Style::of(dir);
            let sep = |c| style.is_sep(c);
            let dir = dir.trim_end_matches(sep);
            let (parent, last) = dir.rsplit_once(sep)?;
            (last == name && !parent.is_empty()).then(|| parent.to_owned())
        };
        under(self.database_folder_dir.as_ref(), "sqlite")
            .or_else(|| under(self.cover_folder_dir.as_ref(), "covers"))
    }
}

/// `--map-path FROM=TO` rules for paths the old server wrote, which name
/// folders as the old machine or container saw them.
#[derive(Debug, Clone, Default)]
pub struct PathMap(Vec<(String, PathBuf)>);

/// A mapped path and whether the import is sure of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapped {
    pub path: PathBuf,
    /// Found by replacing the old data directory with the one on this
    /// machine, rather than by a rule or as written.
    pub guessed: bool,
}

impl PathMap {
    /// Splits each pair at the first `=`, because an old path is a prefix
    /// the user copies from the old config and rarely contains one.
    pub fn parse(pairs: &[String]) -> Result<Self> {
        let mut rules = Vec::new();
        for pair in pairs {
            let Some((from, to)) = pair.split_once('=') else {
                bail!("--map-path {pair:?} is not FROM=TO");
            };
            if from.is_empty() || to.is_empty() {
                bail!("--map-path {pair:?} needs both an old and a new path");
            }
            // The server only accepts absolute library folders, so a
            // relative target is resolved here, where the user typed it.
            rules.push((from.to_owned(), std::path::absolute(to)?));
        }
        // The longest prefix wins, so a rule for a subfolder beats one for
        // its parent.
        rules.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        Ok(Self(rules))
    }

    /// Where `old` is on this machine. `data_dir` is the old data directory
    /// as found here, and `old_data_dir` as the old server saw it.
    pub fn map(&self, old: &str, data_dir: &Path, old_data_dir: Option<&str>) -> Option<Mapped> {
        let style = if is_absolute(old) {
            Style::of(old)
        } else {
            // A relative path has no drive or root to tell by. Without an
            // old data directory both slashes split, as they always did.
            old_data_dir.map_or(Style::Windows, Style::of)
        };
        for (from, to) in &self.0 {
            if let Some((tail, style)) = strip_prefix(old, from, style) {
                return Some(Mapped {
                    path: join_tail(to, tail, style),
                    guessed: false,
                });
            }
        }
        if !is_absolute(old) {
            // The old server resolved relative paths against its data
            // directory.
            return Some(Mapped {
                path: join_tail(data_dir, old, style),
                guessed: false,
            });
        }
        if Path::new(old).exists() {
            return Some(Mapped {
                path: PathBuf::from(old),
                guessed: false,
            });
        }
        let (tail, style) = strip_prefix(old, old_data_dir?, style)?;
        Some(Mapped {
            path: join_tail(data_dir, tail, style),
            guessed: true,
        })
    }
}

fn is_windows_path(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with("\\\\") || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || is_windows_path(path)
}

/// Which characters split the folders of a path the old server wrote. On
/// Unix a backslash is an ordinary character of a folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Windows,
    Unix,
}

impl Style {
    fn of(path: &str) -> Self {
        if is_windows_path(path) {
            Self::Windows
        } else {
            Self::Unix
        }
    }

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == Self::Windows && c == '\\')
    }
}

/// The rest of `path` after `prefix`, if `prefix` ends at a folder
/// boundary, and the style that rest splits by. A Windows prefix compares
/// without case and with either slash even when `path` does not look like a
/// Windows path, as a UNC path written with forward slashes does not.
fn strip_prefix<'a>(path: &'a str, prefix: &str, style: Style) -> Option<(&'a str, Style)> {
    let style = if is_windows_path(prefix) {
        Style::Windows
    } else {
        style
    };
    let sep = |c| style.is_sep(c);
    let prefix = prefix.trim_end_matches(sep);
    let head = path.get(..prefix.len())?;
    let same = if is_windows_path(prefix) {
        head.chars()
            .zip(prefix.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b) || (sep(a) && sep(b)))
    } else {
        head == prefix
    };
    let tail = &path[prefix.len()..];
    (same && (tail.is_empty() || tail.starts_with(sep))).then_some((tail, style))
}

/// Joins the folders of an old relative tail. `.` and `..` are dropped
/// rather than followed, so a crafted config cannot point the import above
/// the folder it was mapped to. Each folder is read again as a path of this
/// machine, because on Windows a backslash or drive inside an old Unix name
/// would otherwise act as one when pushed.
fn join_tail(base: &Path, tail: &str, style: Style) -> PathBuf {
    let mut out = base.to_owned();
    for part in tail.split(|c| style.is_sep(c)) {
        for component in Path::new(part).components() {
            if let Component::Normal(name) = component {
                out.push(name);
            }
        }
    }
    out
}

/// Where the old config is, given the old data directory found on disk.
pub fn beside(data_dir: &Path) -> Option<PathBuf> {
    [data_dir.join("config").join(FILE), data_dir.join(FILE)]
        .into_iter()
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
        "version": "0.6.17",
        "jwtsecret": "JWT-SECRET-VALUE",
        "md5secret": "MD5-SECRET-VALUE",
        "sqliteType": "sqlite3",
        "dbUseDefaultPath": true,
        "databaseFolderDir": "/usr/src/kikoeru/sqlite",
        "coverFolderDir": "/usr/src/kikoeru/covers",
        "auth": true,
        "rootFolders": [
            {"name": "local", "path": "/usr/src/kikoeru/VoiceWork"},
            {"name": "cloud", "type": "openlist", "url": "http://ol:5244",
             "basePath": "/cloud/works", "username": "kikoeru",
             "password": "OPENLIST-PASSWORD"},
            {"path": "/no/name"},
            "not an object"
        ]
    }"#;

    #[test]
    fn keeps_only_the_whitelisted_fields() {
        let c: LegacyConfig = serde_json::from_str(CONFIG).unwrap();
        assert_eq!(c.version.as_deref(), Some("0.6.17"));
        assert_eq!(c.root_folders.len(), 2, "{:?}", c.root_folders);
        assert_eq!(c.root_folders[0].kind, "local");
        assert_eq!(c.root_folders[1].username.as_deref(), Some("kikoeru"));
        let shown = format!("{c:?}{}", serde_json::to_string(&c).unwrap());
        for secret in ["JWT-SECRET-VALUE", "MD5-SECRET-VALUE", "OPENLIST-PASSWORD"] {
            assert!(!shown.contains(secret), "{secret} leaked: {shown}");
        }
        assert_eq!(c.old_data_dir().as_deref(), Some("/usr/src/kikoeru"));
        assert!(c.check_database_type().is_ok());
        let mysql = LegacyConfig {
            sqlite_type: Some("mysql".into()),
            ..c
        };
        assert!(mysql.check_database_type().is_err());
    }

    #[test]
    fn a_wrong_whitelisted_value_does_not_echo_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, r#"{"auth": "yes", "jwtsecret": "JWT-SECRET-VALUE"}"#).unwrap();
        let err = format!("{:#}", LegacyConfig::read(&path).unwrap_err());
        assert!(err.contains("not a kikoeru config.json"), "{err}");
        assert!(!err.contains("JWT-SECRET-VALUE"), "{err}");
    }

    #[test]
    fn maps_old_paths_onto_this_machine() {
        let map = PathMap::parse(&[
            "/usr/src/kikoeru/VoiceWork=/srv/voice".into(),
            "/usr/src/kikoeru/VoiceWork/RJ=/srv/rj".into(),
            r"C:\kikoeru\VoiceWork=/mnt/win".into(),
        ])
        .unwrap();
        let data = Path::new("/srv/old");
        let old = Some("/usr/src/kikoeru");
        let mapped = |p: &str| map.map(p, data, old);

        assert_eq!(
            mapped("/usr/src/kikoeru/VoiceWork/a").unwrap().path,
            Path::new("/srv/voice/a")
        );
        assert_eq!(
            mapped("/usr/src/kikoeru/VoiceWork/RJ/b").unwrap().path,
            Path::new("/srv/rj/b"),
            "the longer rule wins"
        );
        // A rule only matches whole folder names.
        assert_eq!(
            mapped("/usr/src/kikoeru/VoiceWork2").unwrap(),
            Mapped {
                path: "/srv/old/VoiceWork2".into(),
                guessed: true
            }
        );
        assert_eq!(
            mapped(r"c:\KIKOERU\voicework\x\y").unwrap().path,
            Path::new("/mnt/win/x/y"),
            "Windows prefixes ignore case and slash style"
        );
        assert_eq!(
            mapped("./covers").unwrap(),
            Mapped {
                path: "/srv/old/covers".into(),
                guessed: false
            }
        );
        assert_eq!(
            mapped("/usr/src/kikoeru/../../etc").unwrap().path,
            Path::new("/srv/old/etc"),
            "dot segments are dropped"
        );
        assert_eq!(mapped("/elsewhere/x"), None);
        let relative = PathMap::parse(&["/old=voice".into()]).unwrap();
        let to = relative.map("/old/a", data, old).unwrap().path;
        assert!(
            to.is_absolute() && to.ends_with("voice/a"),
            "{}",
            to.display()
        );
        // On Unix a backslash belongs to the folder name.
        let unix = PathMap::parse(&["/old=/mnt".into(), "/old/voice=/elsewhere".into()]).unwrap();
        assert_eq!(
            unix.map(r"/old/voice\archive", data, old).unwrap().path,
            Path::new("/mnt").join(r"voice\archive")
        );
        assert_eq!(
            mapped(r"a\b").unwrap().path,
            Path::new("/srv/old").join(r"a\b"),
            "a relative path follows the old data directory's style"
        );
        assert_eq!(
            map.map(r"a\b", data, None).unwrap().path,
            Path::new("/srv/old/a/b"),
            "without one both slashes split"
        );
        let unc = PathMap::parse(&[r"\\server\share=/mnt/share".into()]).unwrap();
        assert_eq!(
            unc.map("//server/share/VoiceWork", data, Some(r"C:\kikoeru"))
                .unwrap()
                .path,
            Path::new("/mnt/share/VoiceWork"),
            "a Windows rule matches a UNC path written with forward slashes"
        );
        assert_eq!(
            PathMap::default()
                .map("//server/share/VoiceWork", data, Some(r"\\server\share"))
                .unwrap(),
            Mapped {
                path: "/srv/old/VoiceWork".into(),
                guessed: true
            },
            "so does a UNC data directory"
        );
        assert!(PathMap::parse(&["nope".into()]).is_err());
        assert!(PathMap::parse(&["=x".into()]).is_err());
    }

    /// On Windows a backslash or drive inside an old Unix folder name would
    /// climb out of the mapped folder if pushed as it is.
    #[test]
    fn a_mapped_path_stays_inside_its_folder() {
        let base = Path::new("/mnt");
        for tail in [r"/..\outside", r"/C:\x", "/../x", r"\..\..\x", "/./a/../b"] {
            for style in [Style::Unix, Style::Windows] {
                let out = join_tail(base, tail, style);
                let rest = out.strip_prefix(base).unwrap();
                assert!(
                    rest.components().all(|c| matches!(c, Component::Normal(_))),
                    "{tail:?} {style:?} -> {}",
                    out.display()
                );
            }
        }
    }
}
