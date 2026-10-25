//! The old media library folders, mapped onto this machine, and the
//! `[[roots]]` entries that `--write-config` writes for them.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Serialize;

use super::{
    legacy::Work,
    legacy_config::{LegacyConfig, PathMap},
    source::{Located, Origin},
};
use crate::private::PrivateFile;

/// An old media library as the old config describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OldRoot {
    pub name: String,
    /// `local` or `openlist`. Other types are listed so the user sees them,
    /// but get no entry.
    pub kind: String,
    /// The folder as the old server saw it.
    pub old_path: Option<String>,
    /// Where that folder is on this machine, when the import found it.
    pub path: Option<Located>,
    pub url: Option<String>,
    pub base_path: Option<String>,
    pub username: Option<String>,
    /// For a local folder: how many of the sampled works' directories it
    /// holds. Filled in by [`spot_check`].
    pub sample: Option<Sample>,
}

/// The result of looking for some of a library's works in its folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub checked: u64,
    pub found: u64,
}

impl Sample {
    /// Fewer than half found: probably the wrong folder, or a disk that is
    /// not mounted.
    pub fn is_low(&self) -> bool {
        self.found * 2 < self.checked
    }
}

/// How many works a spot check looks for in each folder.
const SAMPLE_SIZE: usize = 20;

/// Looks for up to 20 of each local library's works in the folder it
/// was mapped to, spread evenly over the old work IDs, so a wrong folder
/// is caught before the config points the scanner at it.
pub fn spot_check(old: &mut [OldRoot], works: &[Work]) {
    // The same rule as the import's: a library written on Windows uses `\`.
    let windows = super::plan::windows_dirs_in(works);
    for root in old.iter_mut().filter(|r| r.kind == "local") {
        let Some(path) = &root.path else {
            continue;
        };
        let dirs: Vec<String> = works
            .iter()
            .filter(|w| w.root_folder == root.name)
            .map(|w| {
                if windows {
                    w.dir.replace('\\', "/")
                } else {
                    w.dir.clone()
                }
            })
            .collect();
        if dirs.is_empty() {
            continue;
        }
        let step = dirs.len().div_ceil(SAMPLE_SIZE);
        let sampled: Vec<&String> = dirs.iter().step_by(step).collect();
        let found = sampled
            .iter()
            .filter(|d| path.path.join(d).is_dir())
            .count();
        root.sample = Some(Sample {
            checked: sampled.len() as u64,
            found: found as u64,
        });
    }
}

/// The old config's media libraries. `data_dir` is the old data directory
/// on this machine.
pub fn old_roots(settings: &LegacyConfig, data_dir: Option<&Path>, map: &PathMap) -> Vec<OldRoot> {
    let old_data_dir = settings.old_data_dir();
    settings
        .root_folders
        .iter()
        .map(|r| {
            let path = r.path.as_deref().and_then(|old| {
                let mapped = map.map(old, data_dir?, old_data_dir.as_deref())?;
                // A folder that is not there was mapped wrongly or is on a
                // disk that is not mounted. Either way it is not found.
                mapped.path.is_dir().then_some(Located {
                    path: mapped.path,
                    origin: if mapped.guessed {
                        Origin::Guessed
                    } else {
                        Origin::Config("rootFolders")
                    },
                })
            });
            OldRoot {
                name: r.name.clone(),
                kind: r.kind.clone(),
                old_path: r.path.clone(),
                path,
                url: r.url.clone(),
                base_path: r.base_path.clone(),
                username: r.username.clone(),
                sample: None,
            }
        })
        .collect()
}

#[derive(Serialize)]
struct RootsFile {
    roots: Vec<RootOut>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RootOut {
    Local {
        id: String,
        display_name: String,
        path: PathBuf,
    },
    OpenList {
        id: String,
        display_name: String,
        url: String,
        base_path: String,
        username: String,
        password_file: PathBuf,
    },
}

impl RootOut {
    fn id(&self) -> &str {
        match self {
            Self::Local { id, .. } | Self::OpenList { id, .. } => id,
        }
    }

    /// What the entry points at, without its name.
    fn place(&self) -> String {
        match self {
            Self::Local { path, .. } => format!("local {}", path.display()),
            Self::OpenList {
                url,
                base_path,
                username,
                ..
            } => format!("openlist {url} {base_path} {username}"),
        }
    }
}

/// `[[roots]]` for every old root folder mapped with `--root`. OpenList
/// passwords are not in the old config the import reads, so each entry
/// points at `secrets/openlist-<id>.password` under `data_dir`.
///
/// An entry the server could not start with, a local one without a folder
/// on this machine or an OpenList one whose password file does not exist
/// yet, is written commented out under a TODO line. The text can then go
/// straight into config.toml without stopping the server from starting.
pub fn render(roots: &BTreeMap<String, String>, old: &[OldRoot], data_dir: &Path) -> String {
    // The server reads password_file as written, from wherever it starts.
    let data_dir = &std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_owned());
    // In old name order, so the first library under a shared ID wins.
    let mut entries: Vec<(Option<String>, RootOut)> = Vec::new();
    let mut notes = Vec::new();
    for (old_name, id) in roots {
        let Some(r) = old.iter().find(|r| &r.name == old_name) else {
            notes.push(format!(
                "{old_name} ({id}) is not in the old config.json; add its [[roots]] entry by hand"
            ));
            continue;
        };
        let display_name = r.name.clone();
        match r.kind.as_str() {
            "local" => match (&r.path, &r.old_path) {
                (Some(p), _) => {
                    let entry = RootOut::Local {
                        id: id.clone(),
                        display_name,
                        path: p.path.clone(),
                    };
                    // A guessed folder may be the wrong one, and a live
                    // entry would scan it, so it stays commented out.
                    if p.origin == Origin::Guessed {
                        let reason = format!(
                            "{id}: check the guessed folder {}, then uncomment",
                            p.path.display()
                        );
                        entries.push((Some(reason), entry));
                    } else {
                        entries.push((None, entry));
                    }
                }
                (None, old_path) => {
                    let reason = match old_path {
                        Some(p) => format!(
                            "{id}: {p} was not found on this machine; set the folder of this library"
                        ),
                        None => format!("{id}: set the folder of this library"),
                    };
                    let entry = RootOut::Local {
                        id: id.clone(),
                        display_name,
                        path: PathBuf::from(old_path.clone().unwrap_or_default()),
                    };
                    entries.push((Some(reason), entry));
                }
            },
            "openlist" => {
                let password_file = data_dir
                    .join("secrets")
                    .join(format!("openlist-{id}.password"));
                if r.username.is_none() {
                    notes.push(format!("{id}: the old config has no username; set it"));
                }
                let ready_now = password_file.is_file();
                let reason = format!(
                    "{id}: write the OpenList password into {} (mode 0600), then uncomment",
                    password_file.display()
                );
                let entry = RootOut::OpenList {
                    id: id.clone(),
                    display_name,
                    url: r.url.clone().unwrap_or_default(),
                    base_path: r.base_path.clone().unwrap_or_default(),
                    username: r.username.clone().unwrap_or_default(),
                    password_file,
                };
                if ready_now {
                    entries.push((None, entry));
                } else {
                    entries.push((Some(reason), entry));
                }
            }
            other => notes.push(format!(
                "{old_name}: the old library type {other:?} has no counterpart and was left out"
            )),
        }
    }
    // Several old libraries may share one new ID. The same place is
    // written once. Different places under one ID would stop the server
    // from starting, so all but the first are left commented out.
    let mut places = BTreeMap::<String, String>::new();
    let mut ready: Vec<RootOut> = Vec::new();
    let mut pending = Vec::new();
    for (reason, entry) in entries {
        let place = entry.place();
        match places.get(entry.id()) {
            None => {
                places.insert(entry.id().to_owned(), place);
                match reason {
                    None => ready.push(entry),
                    Some(r) => pending.push((r, entry)),
                }
            }
            // The same place under the same ID is written once.
            Some(first) if *first == place => {}
            Some(_) => pending.push((
                format!(
                    "{}: another old library already uses this ID for another place; \
                     give this one its own ID",
                    entry.id()
                ),
                entry,
            )),
        }
    }
    let mut text = String::from(
        "# Media libraries from the old kikoeru config.json, written by kikoeru-neo.\n",
    );
    text.push_str(SCAN_HINTS);
    for line in &notes {
        let _ = writeln!(text, "# TODO {}", one_line(line));
    }
    if !ready.is_empty() {
        text.push('\n');
        text.push_str(&toml::to_string(&RootsFile { roots: ready }).unwrap_or_default());
    }
    for (reason, entry) in pending {
        let _ = write!(text, "\n# TODO {}\n", one_line(&reason));
        let entry = toml::to_string(&RootsFile { roots: vec![entry] }).unwrap_or_default();
        for line in entry.lines() {
            let _ = writeln!(text, "# {line}");
        }
    }
    text
}

/// The per-root scan settings the old config has no counterpart for. They
/// are left to their defaults, so the hint is all that says they exist.
const SCAN_HINTS: &str = "\
# Each library can also take (book/src/contract.md, \"自动扫描和目录监听\"):
#   auto_scan_interval = \"6h\"  scan on this interval. Off when left out,
#                               at least 30m for openlist, 5m for local.
#   watch = true                local only: scan when files change. Left
#                               out, it is off for serve, on in the desktop.
";

/// Old paths and names can hold line breaks, which would end a TOML
/// comment and leave the rest as broken config.
fn one_line(text: &str) -> String {
    text.replace('\n', "\\n").replace('\r', "\\r")
}

/// Writes `text` to a new 0600 file. An existing file is never replaced,
/// because it may be the user's config.toml.
pub fn write(path: &Path, text: &str) -> Result<()> {
    use io::Write as _;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn spot_checks_a_spread_of_works_in_each_local_folder() {
        let dir = tempfile::tempdir().unwrap();
        let works: Vec<Work> = (0..100)
            .map(|i| Work {
                id: 100_000 + i,
                root_folder: "Old".into(),
                dir: format!("RJ{}", 100_000 + i),
                title: String::new(),
                circle_id: 1,
                nsfw: None,
                release: None,
                dl_count: None,
                price: None,
                rate_count: None,
                rate_average: None,
                rate_count_detail: None,
                rank: None,
                memo: None,
                is_custom_meta: None,
                created_at: None,
                updated_at: None,
            })
            .collect();
        // Only the first 30 works are in this folder.
        for w in &works[..30] {
            fs::create_dir(dir.path().join(&w.dir)).unwrap();
        }
        let mut old = vec![OldRoot {
            name: "Old".into(),
            kind: "local".into(),
            old_path: None,
            path: Some(Located {
                path: dir.path().to_owned(),
                origin: Origin::Given,
            }),
            url: None,
            base_path: None,
            username: None,
            sample: None,
        }];
        spot_check(&mut old, &works);
        let sample = old[0].sample.unwrap();
        assert_eq!(
            sample,
            Sample {
                checked: 20,
                found: 6
            },
            "every fifth work"
        );
        assert!(sample.is_low());
        for w in &works[30..] {
            fs::create_dir(dir.path().join(&w.dir)).unwrap();
        }
        spot_check(&mut old, &works);
        assert!(!old[0].sample.unwrap().is_low());

        // A library written on Windows names nested folders with `\`.
        let windows: Vec<Work> = works
            .iter()
            .map(|w| Work {
                dir: format!(r"nested\{}", w.dir),
                ..w.clone()
            })
            .collect();
        for w in &works {
            fs::create_dir_all(dir.path().join("nested").join(&w.dir)).unwrap();
        }
        spot_check(&mut old, &windows);
        assert_eq!(old[0].sample.unwrap().found, 20);
    }

    #[test]
    fn writes_roots_the_server_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let voice = dir.path().join("voice");
        fs::create_dir(&voice).unwrap();
        let settings: LegacyConfig = serde_json::from_str(
            r#"{"databaseFolderDir": "/usr/src/kikoeru/sqlite",
                "rootFolders": [
                  {"name": "Lo=cal", "path": "/old/voice"},
                  {"name": "gone", "path": "/old/gone"},
                  {"name": "cloud", "type": "openlist", "url": "http://ol:5244",
                    "basePath": "/115", "username": "u", "password": "SECRET"},
                  {"name": "odd", "type": "webdav"}
                ]}"#,
        )
        .unwrap();
        let map = PathMap::parse(&[format!("/old/voice={}", voice.display())]).unwrap();
        let old = old_roots(&settings, Some(dir.path()), &map);
        assert_eq!(old[0].path.as_ref().unwrap().path, voice);
        assert_eq!(old[1].path, None);

        let roots = BTreeMap::from([
            ("Lo=cal".to_owned(), "local".to_owned()),
            ("gone".to_owned(), "gone".to_owned()),
            ("cloud".to_owned(), "cloud".to_owned()),
            ("odd".to_owned(), "odd".to_owned()),
        ]);
        let data = Path::new("/var/lib/kikoeru-neo");
        let text = render(&roots, &old, data);
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("#   auto_scan_interval = \"6h\""), "{text}");
        assert!(
            text.contains("# TODO gone: /old/gone was not found"),
            "{text}"
        );
        assert!(text.contains("# path = \"/old/gone\""), "{text}");
        assert!(
            text.contains("/var/lib/kikoeru-neo/secrets/openlist-cloud.password"),
            "{text}"
        );
        assert!(text.contains("\"webdav\" has no counterpart"), "{text}");
        // Only the complete entry is live, so the server starts with it.
        let config = Config::parse(&text, Path::new("roots.toml")).unwrap();
        let ids: Vec<&str> = config.roots.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["local"]);

        // With its password file in place, the OpenList entry is live too.
        let secrets = dir.path().join("secrets");
        fs::create_dir(&secrets).unwrap();
        fs::write(secrets.join("openlist-cloud.password"), "pw").unwrap();
        let text = render(&roots, &old, dir.path());
        let config = Config::parse(&text, Path::new("roots.toml")).unwrap();
        let ids: Vec<&str> = config.roots.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["local", "cloud"]);

        // Two old libraries under one ID: the same place is written once,
        // another place is left commented out.
        let other = dir.path().join("other");
        fs::create_dir(&other).unwrap();
        let mut twins = old.clone();
        for (name, path) in [("A", &voice), ("B", &voice), ("C", &other)] {
            twins.push(OldRoot {
                name: name.into(),
                kind: "local".into(),
                old_path: None,
                path: Some(Located {
                    path: path.clone(),
                    origin: Origin::Given,
                }),
                url: None,
                base_path: None,
                username: None,
                sample: None,
            });
        }
        let shared = BTreeMap::from([
            ("A".to_owned(), "same".to_owned()),
            ("B".to_owned(), "same".to_owned()),
            ("C".to_owned(), "same".to_owned()),
        ]);
        let text = render(&shared, &twins, dir.path());
        let config = Config::parse(&text, Path::new("roots.toml")).unwrap();
        assert_eq!(config.roots.len(), 1, "{text}");
        assert!(text.contains("give this one its own ID"), "{text}");

        // A guessed folder is not scanned until the user confirms it, and
        // a line break in an old path cannot break out of a comment.
        let odd = vec![
            OldRoot {
                name: "guess".into(),
                kind: "local".into(),
                old_path: None,
                path: Some(Located {
                    path: voice.clone(),
                    origin: Origin::Guessed,
                }),
                url: None,
                base_path: None,
                username: None,
                sample: None,
            },
            OldRoot {
                name: "nl".into(),
                kind: "local".into(),
                old_path: Some("/gone\nstray = 1".into()),
                path: None,
                url: None,
                base_path: None,
                username: None,
                sample: None,
            },
        ];
        let ids = BTreeMap::from([
            ("guess".to_owned(), "guess".to_owned()),
            ("nl".to_owned(), "nl".to_owned()),
        ]);
        let text = render(&ids, &odd, dir.path());
        let config = Config::parse(&text, Path::new("roots.toml")).unwrap();
        assert!(config.roots.is_empty(), "{text}");
        assert!(text.contains("check the guessed folder"), "{text}");

        // The first library under a shared ID wins even when a later one
        // is incomplete.
        let mut ordered = twins.clone();
        ordered.push(OldRoot {
            name: "ZZ".into(),
            kind: "local".into(),
            old_path: Some("/missing".into()),
            path: None,
            url: None,
            base_path: None,
            username: None,
            sample: None,
        });
        let ids = BTreeMap::from([
            ("A".to_owned(), "same".to_owned()),
            ("ZZ".to_owned(), "same".to_owned()),
        ]);
        let text = render(&ids, &ordered, dir.path());
        let config = Config::parse(&text, Path::new("roots.toml")).unwrap();
        assert_eq!(config.roots.len(), 1, "{text}");

        let out = dir.path().join("roots.toml");
        write(&out, &text).unwrap();
        assert!(write(&out, &text).is_err(), "never replaces a file");
    }
}
