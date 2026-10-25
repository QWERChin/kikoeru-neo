//! Importing an old kikoeru library into the local one. The desktop only
//! merges user data, as "合并到已有的库" in book/src/migration.md describes:
//! the local library scans its own works.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use neo_core::dto::{
    ImportCounts, ImportDiscovered, ImportErrorCode, ImportFailure, ImportIssue, ImportOldRoot,
    ImportPath, ImportPreview, ImportResult, ImportSample, ImportSource, ImportUser,
};
use neo_server::import::{
    config_out::{self, OldRoot},
    error,
    legacy_config::PathMap,
    merge::{self, Merged, UserMap},
    plan::{self, Plan},
    report::Report,
    session::{Options, Session},
    snapshot::Journal,
    source::{self, Given, Located, Source},
};

use crate::local::ADMIN;

/// The old data the page picked, and how far the user has confirmed it.
#[derive(Debug, Clone)]
pub struct Pick {
    pub pick: PathBuf,
    pub accept_guessed: bool,
    pub recover_journal: bool,
}

/// The page's copy of an error: its code, and the whole reason chain.
pub fn failure(err: &anyhow::Error) -> ImportFailure {
    ImportFailure {
        code: error::code(err),
        message: format!("{err:#}"),
    }
}

/// A failure of the shell's own, which has no import code.
pub fn other(message: impl Into<String>) -> ImportFailure {
    ImportFailure {
        code: ImportErrorCode::Other,
        message: message.into(),
    }
}

fn path(located: &Located) -> ImportPath {
    ImportPath {
        path: located.path.to_string_lossy().into_owned(),
        // The same string the import report writes for the origin.
        origin: serde_json::to_value(located.origin)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

fn source_of(source: &Source) -> ImportSource {
    ImportSource {
        db: path(&source.db),
        covers: source.covers.as_ref().map(path),
        config: source.config.as_ref().map(path),
        version: source.settings.as_ref().and_then(|s| s.version.clone()),
    }
}

fn count(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

/// The merge's counts. `roots_added` is filled in after the roots.
pub fn result(merged: Merged) -> ImportResult {
    let c = merged.claimed;
    ImportResult {
        works: count(merged.works),
        favorites: count(c.favorites),
        reviews: count(c.reviews),
        progress: count(c.progress),
        progress_kept: count(c.progress_kept),
        waiting: count(merged.waiting),
        users_created: merged.users_created,
        roots_added: Vec::new(),
    }
}

/// Finds the old data without opening its database.
pub fn discover(pick: PathBuf) -> Result<ImportDiscovered> {
    let source = source::locate(&pick, &Given::default())?;
    Ok(ImportDiscovered {
        pick: pick.to_string_lossy().into_owned(),
        source: source_of(&source),
    })
}

/// The old library read from a fresh snapshot, its plan, and the report
/// the page shows.
pub struct Opened {
    pub plan: Plan,
    pub report: Report,
    pub refusal: Option<Vec<String>>,
}

pub async fn open(pick: Pick, now: i64) -> Result<Opened> {
    let tmp = std::env::temp_dir();
    let options = Options {
        given: Given::default(),
        accept_guessed: pick.accept_guessed,
        journal: if pick.recover_journal {
            Journal::Recover
        } else {
            Journal::Refuse
        },
        tmp: &tmp,
    };
    let session = Session::open(&pick.pick, &options).await?;
    // The old roots, which the merge leaves alone, are only offered as
    // folders for the local library.
    let (plan, mut report, outcome) = plan::build(&session.legacy, None, now);
    report.old_roots = old_roots(&session.source);
    // The page can then warn before a folder with few of its works is added.
    config_out::spot_check(&mut report.old_roots, &session.legacy.works);
    report.source = Some(session.source);
    Ok(Opened {
        plan,
        report,
        refusal: outcome.err().map(|r| r.0),
    })
}

fn old_roots(source: &Source) -> Vec<OldRoot> {
    let Some(settings) = &source.settings else {
        return Vec::new();
    };
    let data_dir = source
        .config
        .as_ref()
        .and_then(|c| source::data_dir_of_config(&c.path));
    config_out::old_roots(settings, data_dir.as_deref(), &PathMap::default())
}

pub fn preview(opened: Opened) -> ImportPreview {
    let report = &opened.report;
    let table = |name: &str| report.tables.get(name).map_or(0, |t| count(t.source));
    let written = |name: &str| report.written.get(name).copied().map_or(0, count);
    ImportPreview {
        source: report
            .source
            .as_ref()
            .map(source_of)
            .expect("open() sets the source"),
        counts: ImportCounts {
            works: table("t_work"),
            users: table("t_user"),
            favorites: written("favorites"),
            reviews: written("legacy_reviews"),
            progress: written("play_progress"),
            works_with_data: count(merge::states_len(&opened.plan)),
        },
        users: opened
            .plan
            .users
            .iter()
            .map(|u| ImportUser {
                name: u.name.clone(),
                is_admin: u.is_admin,
            })
            .collect(),
        old_roots: report
            .old_roots
            .iter()
            .map(|r| ImportOldRoot {
                name: r.name.clone(),
                kind: r.kind.clone(),
                old_path: r.old_path.clone(),
                path: r.path.as_ref().map(path),
                addable: r.kind == "local" && r.path.is_some(),
                sample: r.sample.map(|s| ImportSample {
                    checked: count(s.checked),
                    found: count(s.found),
                }),
            })
            .collect(),
        issues: report
            .issues
            .iter()
            .map(|i| ImportIssue {
                at: i.at.clone(),
                reason: i.reason.clone(),
            })
            .collect(),
        refusal: opened.refusal,
    }
}

/// Every old user in `to_me` merges into the local account. The others
/// keep their names.
pub fn users(to_me: &[String], plan: &Plan) -> Result<UserMap> {
    let pairs: Vec<String> = to_me.iter().map(|old| format!("{old}={ADMIN}")).collect();
    merge::user_map(&pairs, plan)
}

/// The old local roots named in `names`, as their names and folders,
/// minus the folders the library already has. Checked before the merge, so
/// a wrong name changes nothing.
pub fn roots_to_add(
    report: &Report,
    names: &[String],
    existing: &[String],
) -> Result<Vec<(String, String)>> {
    let mut found: Vec<(String, String)> = Vec::new();
    for name in names {
        let Some(root) = report.old_roots.iter().find(|r| &r.name == name) else {
            bail!("旧库里没有媒体库 {name}");
        };
        let Some(path) = root.path.as_ref().filter(|_| root.kind == "local") else {
            bail!("旧媒体库 {name} 不是能找到位置的本地文件夹");
        };
        let Some(path) = path.path.to_str() else {
            bail!("旧媒体库 {name} 的路径不是有效的文本");
        };
        // Path equality, so `/m/a/` and `/m/./a` match `/m/a`.
        let same = |other: &String| Path::new(other) == Path::new(path);
        let known = existing.iter().any(same) || found.iter().any(|(_, p)| same(p));
        if !known {
            found.push((name.clone(), path.to_owned()));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use neo_server::import::{
        plan::UserRow,
        source::{Located, Origin},
    };

    use super::*;

    fn root(name: &str, kind: &str, path: Option<&str>) -> OldRoot {
        OldRoot {
            name: name.to_owned(),
            kind: kind.to_owned(),
            old_path: None,
            path: path.map(|p| Located {
                path: p.into(),
                origin: Origin::Default,
            }),
            url: None,
            base_path: None,
            username: None,
            sample: None,
        }
    }

    const OLD: &str = include_str!("../../../crates/neo-server/tests/fixtures/legacy_0614.sql");

    /// An old data directory with the fixture's database and a config.json
    /// whose one local root exists on this machine.
    async fn old_install(dir: &Path) -> PathBuf {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let old = dir.join("old");
        std::fs::create_dir_all(old.join("sqlite")).unwrap();
        std::fs::create_dir_all(old.join("config")).unwrap();
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(old.join("sqlite/db.sqlite3"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(OLD).execute(&pool).await.unwrap();
        pool.close().await;
        let media = dir.join("media");
        std::fs::create_dir(&media).unwrap();
        let config = serde_json::json!({
            "rootFolders": [{"name": "Old", "path": media}],
        });
        std::fs::write(old.join("config/config.json"), config.to_string()).unwrap();
        old
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merges_an_old_library_into_the_local_account() {
        let dir = tempfile::tempdir().unwrap();
        let old = old_install(dir.path()).await;
        let found = discover(old.clone()).unwrap();
        assert_eq!(
            found.source.db.path,
            old.join("sqlite/db.sqlite3").to_string_lossy()
        );
        assert_eq!(found.source.db.origin, "default");

        let pick = Pick {
            pick: old,
            accept_guessed: false,
            recover_journal: false,
        };
        let shown = preview(open(pick.clone(), 0).await.unwrap());
        assert!(shown.refusal.is_none(), "{:?}", shown.refusal);
        assert_eq!(shown.counts.works_with_data, 2);
        assert_eq!(shown.counts.works, 2);
        let names: Vec<&str> = shown.users.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["admin", "guest"]);
        assert_eq!(shown.old_roots.len(), 1);
        assert!(shown.old_roots[0].addable, "{:?}", shown.old_roots);
        assert!(shown.old_roots[0].sample.is_some(), "{:?}", shown.old_roots);

        let data = dir.path().join("data");
        let server = tokio::spawn(neo_server::embed::start(neo_server::embed::Options {
            data_dir: data.clone(),
            ffmpeg: None,
            ffprobe: None,
            web: None,
        }))
        .await
        .unwrap()
        .unwrap();
        server
            .ensure_admin(ADMIN, "a long random password")
            .await
            .unwrap();
        let opened = open(pick, 0).await.unwrap();
        let users = users(&["admin".into()], &opened.plan).unwrap();
        let old_roots = ["Old".to_owned()];
        let roots = roots_to_add(&opened.report, &old_roots, &[]).unwrap();
        assert_eq!(roots.len(), 1);
        let imported = result(server.merge_legacy(&opened.plan, &users).await.unwrap());

        // The old folder becomes a database root through the API, and a
        // second import skips it.
        let token = server.session_token(ADMIN).await.unwrap();
        let api = crate::roots::Api::new(server.port(), token).unwrap();
        let (name, path) = &roots[0];
        let added = api.add_local(name, path).await.unwrap();
        assert_eq!(added.source, neo_core::dto::RootSource::Db);
        assert_eq!(added.display_name, "Old");
        let listed = api.list().await.unwrap();
        assert!(listed.iter().any(|r| r.id == added.id));
        let paths = crate::roots::local_paths(&listed);
        assert!(
            roots_to_add(&opened.report, &old_roots, &paths)
                .unwrap()
                .is_empty()
        );
        server.stop().await.unwrap();
        // The local library has no works yet, so everything waits for a
        // scan, and only the user not merged into the local account gets
        // an account of its own.
        assert_eq!(imported.works, 2);
        assert_eq!(imported.waiting, 2);
        assert_eq!(imported.users_created, ["guest"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switching_mode_waits_for_the_old_library_read_and_merge() {
        use std::sync::{Arc, Mutex, atomic::AtomicU64};
        use std::time::Duration;
        use tauri::{
            Manager,
            test::{mock_builder, mock_context, noop_assets},
        };
        use tokio::{sync::Barrier, time::timeout};

        let dir = tempfile::tempdir().unwrap();
        let old = old_install(dir.path()).await;
        let server = tokio::spawn(neo_server::embed::start(neo_server::embed::Options {
            data_dir: dir.path().join("data"),
            ffmpeg: None,
            ffprobe: None,
            web: None,
        }))
        .await
        .unwrap()
        .unwrap();
        server
            .ensure_admin(ADMIN, "a long random password")
            .await
            .unwrap();
        let health = format!("http://127.0.0.1:{}/api/health", server.port());
        let barrier = Arc::new(Barrier::new(2));
        let app = mock_builder()
            .manage(crate::Shell {
                config_file: dir.path().join("desktop.json"),
                config: Mutex::new(crate::DesktopConfig {
                    mode: Some(crate::Mode::Local),
                    server_url: None,
                }),
                server: tokio::sync::Mutex::new(Some(server)),
                transition: tokio::sync::Mutex::new(()),
                import_pick: Mutex::new(Some(old.clone())),
                import_read_barrier: Some(barrier.clone()),
                last_error: Mutex::new(None),
                windows: AtomicU64::new(0),
                web: Arc::new(|_| None),
            })
            .build(mock_context(noop_assets()))
            .unwrap();
        tauri::WebviewWindowBuilder::new(
            &app,
            "local",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .build()
        .unwrap();
        let importing = crate::import_run(
            app.handle().clone(),
            old,
            false,
            false,
            vec!["admin".into()],
            vec!["Old".into()],
        );
        tokio::pin!(importing);
        timeout(Duration::from_secs(10), async {
            tokio::select! {
                result = &mut importing => panic!("import finished before the read barrier: {result:?}"),
                _ = barrier.wait() => {}
            }
        }).await.unwrap();

        let switching = crate::switch_mode(app.handle().clone());
        tokio::pin!(switching);
        let early_switch = timeout(Duration::from_millis(100), &mut switching).await;
        let waited = early_switch.is_err();
        let kept_window =
            app.get_webview_window("local").is_some() && app.get_webview_window("main-0").is_none();
        let healthy = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(&health)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());

        // Release the reader and finish both commands before asserting,
        // including on the regression path where switching stopped early.
        let finish_switch = async {
            match early_switch {
                Ok(result) => result,
                Err(_) => switching.await,
            }
        };
        let (_, imported, switched) = timeout(Duration::from_secs(10), async {
            tokio::join!(barrier.wait(), &mut importing, finish_switch)
        })
        .await
        .unwrap();
        switched.unwrap();
        assert!(
            waited,
            "switch_mode completed while the old library read was paused: {imported:?}"
        );
        assert!(
            kept_window,
            "the import's window was destroyed during the read"
        );
        assert!(healthy, "the local backend stopped during the read");
        let imported = imported.unwrap();
        assert_eq!(imported.works, 2);
        assert_eq!(imported.waiting, 2);
        assert_eq!(imported.users_created, ["guest"]);
        assert_eq!(imported.roots_added.len(), 1);
        let shell = app.state::<crate::Shell>();
        assert!(shell.server.lock().await.is_none());
        assert_eq!(shell.config.lock().unwrap().mode, None);
        assert!(app.get_webview_window("main-0").is_some());
    }

    #[test]
    fn a_failure_reaches_the_page_with_its_code() {
        use anyhow::Context;
        let err = Err::<(), _>(anyhow::Error::from(error::ImportError::new(
            ImportErrorCode::TooOld,
            "from 0.6.13",
        )))
        .context("reading the old library")
        .unwrap_err();
        assert_eq!(
            serde_json::to_value(failure(&err)).unwrap(),
            serde_json::json!({"code": "too_old", "message": "reading the old library: from 0.6.13"})
        );
        assert_eq!(
            failure(&anyhow::anyhow!("disk full")).code,
            ImportErrorCode::Other
        );
        assert_eq!(other("已经不在本机模式").code, ImportErrorCode::Other);
    }

    #[test]
    fn only_found_local_roots_are_added_once() {
        let mut report = Report::new(None);
        report.old_roots = vec![
            root("a", "local", Some("/m/a")),
            root("b", "local", Some("/m/b")),
            root("same", "local", Some("/m/a")),
            root("spelled", "local", Some("/m/./a/")),
            root("lost", "local", None),
            root("cloud", "openlist", None),
        ];
        let existing = ["/m/b".to_owned()];
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let paths =
            roots_to_add(&report, &names(&["a", "b", "same", "spelled"]), &existing).unwrap();
        assert_eq!(paths, [("a".to_owned(), "/m/a".to_owned())]);
        for bad in ["lost", "cloud", "missing"] {
            assert!(roots_to_add(&report, &names(&[bad]), &existing).is_err());
        }
    }

    #[test]
    fn chosen_old_users_merge_into_the_local_account() {
        let plan = Plan {
            users: vec![
                UserRow {
                    name: "admin".into(),
                    is_admin: true,
                },
                UserRow {
                    name: "guest".into(),
                    is_admin: false,
                },
            ],
            ..Plan::default()
        };
        let map = users(&["admin".into()], &plan).unwrap();
        assert_eq!(map.get("admin").map(String::as_str), Some(ADMIN));
        assert!(!map.contains_key("guest"));
        assert!(users(&["nobody".into()], &plan).is_err());
    }
}
