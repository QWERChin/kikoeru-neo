//! `import-legacy` on old databases from other kikoeru versions than the one
//! in import_legacy.rs, and on the different places a user may point it at.
//! Every row is made up.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

/// A library from before 0.6.14: `t_work` has no `is_custom_meta`.
const SCHEMA_0613: &str = r#"
CREATE TABLE `knex_migrations` (`id` integer not null primary key autoincrement, `name` varchar(255), `batch` integer, `migration_time` datetime);
CREATE TABLE `t_work` (`id` integer not null primary key autoincrement, `root_folder` varchar(255) not null, `dir` varchar(255) not null, `title` varchar(255) not null, `circle_id` integer not null, `memo` json);
INSERT INTO knex_migrations (name, batch) VALUES ('20240916213303_enable_t_translate_task_auto_update.js', 1);
"#;

/// kikoeru 0.6.17 after a rescan: the memo keeps only `isContainLyric`, and
/// the durations live in `t_track`.
const SCHEMA_0617: &str = r#"
CREATE TABLE `knex_migrations` (`id` integer not null primary key autoincrement, `name` varchar(255), `batch` integer, `migration_time` datetime);
CREATE TABLE `t_circle` (`id` integer not null primary key autoincrement, `name` varchar(255) not null);
CREATE TABLE `t_va` (`id` varchar(255), `name` varchar(255) not null, primary key (`id`));
CREATE TABLE `t_tag` (`id` integer not null primary key autoincrement, `name` varchar(255) not null);
CREATE TABLE `t_work` (`id` integer not null primary key autoincrement, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `root_folder` varchar(255) not null, `dir` varchar(255) not null, `title` varchar(255) not null, `circle_id` integer not null, `nsfw` boolean, `release` varchar(255), `dl_count` integer, `price` integer, `review_count` integer, `rate_count` integer, `rate_average_2dp` float, `rate_count_detail` text, `rank` text, `lyric_status` varchar(255) not null, `original_work_id` bigint not null default '0', `memo` json, `is_custom_meta` integer default '0', `track_max_seq` integer not null default '-1');
CREATE TABLE `t_track` (`id` integer not null primary key autoincrement, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `work_id` bigint not null default '0', `seq` integer not null default '0', `rel_path` varchar(255) not null default '', `ext` varchar(255) not null default '', `file_type` varchar(255) not null default 'unknown', `size` bigint not null default '0', `mtime` bigint not null default '0', `duration` float, `duration_probe_mtime` bigint);
CREATE TABLE `r_tag_work` (`tag_id` integer, `work_id` bigint, primary key (`tag_id`, `work_id`));
CREATE TABLE `r_va_work` (`va_id` varchar(255), `work_id` bigint, primary key (`va_id`, `work_id`));
CREATE TABLE `t_user` (`name` varchar(255) not null, `password` varchar(255) not null, `group` varchar(255) not null, primary key (`name`));
CREATE TABLE `t_review` (`user_name` varchar(255) not null, `work_id` varchar(255) not null, `rating` integer, `review_text` varchar(255), `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `progress` varchar(255), primary key (`user_name`, `work_id`));
CREATE TABLE `t_play_histroy` (`user_name` varchar(255) not null, `work_id` bigint not null, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `state` json not null, primary key (`user_name`, `work_id`));
CREATE TABLE `t_translate_task` (`id` integer not null primary key autoincrement, `work_id` bigint not null default '0');
INSERT INTO knex_migrations (name, batch) VALUES ('20260929120000_audio_cache_t_track.js', 1);
INSERT INTO t_circle VALUES (11, 'Circle A');
INSERT INTO t_user VALUES ('admin', 'oldhash', 'administrator');
INSERT INTO t_work (id, created_at, updated_at, root_folder, dir, title, circle_id, lyric_status, memo) VALUES
  (123456, '2026-01-02 03:04:05', '2026-01-02 03:04:05', 'Old', 'RJ123456', 'Rescanned', 11, '', '{"isContainLyric":false}'),
  (234567, '2026-01-02 03:04:06', '2026-01-02 03:04:06', 'Old', 'RJ234567', 'Not rescanned', 11, '', '{"duration":{"a.mp3":10,"b.mp3":20},"isContainLyric":false}');
INSERT INTO t_track (work_id, seq, rel_path, duration) VALUES
  (123456, 0, 'mp3/01.mp3', 61.5),
  (123456, 1, 'mp3/02.mp3', 90),
  (123456, 2, 'mp3/03.mp3', NULL),
  (234567, 0, 'b.mp3', 25);
INSERT INTO t_review (user_name, work_id, rating, review_text, progress) VALUES
  ('admin', '234567', 5, 'great', 'marked');
INSERT INTO t_play_histroy (user_name, work_id, state) VALUES
  ('admin', 123456, '{"queue":[{"title":"02.mp3","subtitle":"mp3"}],"index":0,"seconds":12.5}');
"#;

struct Source {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Source {
    async fn new(sql: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old");
        fs::create_dir_all(path.join("sqlite")).unwrap();
        fs::create_dir_all(path.join("covers")).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path.join("sqlite/db.sqlite3"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        Self { _dir: dir, path }
    }
}

fn import(source: &Path, target: Option<&Path>, extra: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"));
    cmd.env_remove("KIKOERU_DATA_DIR").env_remove("RUST_LOG");
    if let Some(t) = target {
        cmd.arg("--data-dir").arg(t);
    }
    cmd.arg("import-legacy")
        .arg("--source")
        .arg(source)
        .args(extra)
        .output()
        .unwrap()
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

async fn open(target: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(target.join("neo.sqlite3"))
                .read_only(true),
        )
        .await
        .unwrap()
}

async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn extra(pool: &SqlitePool, code: &str) -> Value {
    let extra: String = sqlx::query_scalar("SELECT extra FROM works WHERE code = ?")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap();
    serde_json::from_str(&extra).unwrap()
}

#[tokio::test]
async fn refuses_a_library_older_than_0_6_14() {
    let source = Source::new(SCHEMA_0613).await;
    let out = import(&source.path, None, &["--dry-run"]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("older than 0.6.14"), "{err}");
    // The message names the user's file, not the private copy.
    let db = source.path.join("sqlite/db.sqlite3");
    assert!(
        err.contains(&format!("{} does not look like", db.display())),
        "{err}"
    );
}

#[tokio::test]
async fn takes_durations_from_t_track_over_the_memo() {
    let source = Source::new(SCHEMA_0617).await;
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");

    let out = import(&source.path, Some(&target), &["--root", "Old=a"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let summary = text(&out.stdout);
    assert!(
        summary.contains("track durations from t_track: 3"),
        "{summary}"
    );
    let db = open(&target).await;
    // A rescanned work has durations only in t_track. A NULL duration is a
    // track that was never probed and stays unknown.
    let rescanned = extra(&db, "RJ123456").await;
    assert_eq!(
        rescanned["legacy_durations"],
        serde_json::json!({"mp3/01.mp3": 61.5, "mp3/02.mp3": 90.0})
    );
    // t_track is newer than the memo where both have a track.
    let kept = extra(&db, "RJ234567").await;
    assert_eq!(
        kept["legacy_durations"],
        serde_json::json!({"a.mp3": 10, "b.mp3": 25.0})
    );
    // The play position needs the duration to be matched later, and t_track
    // supplies it.
    let durations: String = sqlx::query_scalar("SELECT durations FROM import_pending")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(durations.contains("mp3/02.mp3"), "{durations}");
    // A fresh install declares t_review.work_id as a string column, and the
    // review still finds its work.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM legacy_reviews").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);
    db.close().await;
}

#[tokio::test]
async fn names_what_a_foreign_database_lacks() {
    let source = Source::new("CREATE TABLE notes (x text);").await;
    let out = import(&source.path, None, &["--dry-run"]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains("does not look like a kikoeru database"),
        "{err}"
    );
    assert!(
        err.contains("missing knex_migrations, t_circle, t_va, t_tag, t_work,"),
        "{err}"
    );
}

/// Every file under `dir` with its bytes and modification time.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(tree(&path));
        } else {
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            out.push((path.clone(), fs::read(&path).unwrap(), mtime));
        }
    }
    out.sort();
    out
}

fn set_writable(dir: &Path, writable: bool) {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            set_writable(&path, writable);
        }
    }
    let mode = fs::metadata(dir).unwrap().permissions().mode();
    let mode = if writable {
        mode | 0o200
    } else {
        mode & !0o222
    };
    fs::set_permissions(dir, fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn reads_any_folder_of_the_layout_without_writing_to_it() {
    let source = Source::new(SCHEMA_0617).await;
    fs::create_dir(source.path.join("config")).unwrap();
    let before = tree(&source.path);
    // A source the import cannot write to proves it never tries.
    set_writable(&source.path, false);

    let db = source.path.join("sqlite/db.sqlite3");
    for pick in [source.path.clone(), source.path.join("sqlite"), db.clone()] {
        let out = import(&pick, None, &["--dry-run"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert!(
            text(&out.stdout).contains(&format!("old database: {}", db.display())),
            "{}",
            text(&out.stdout)
        );
    }

    // config/ only points at its neighbours, so the user confirms them.
    let config = source.path.join("config");
    let out = import(&config, None, &["--dry-run"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("--accept-guessed"),
        "{}",
        text(&out.stderr)
    );
    let out = import(&config, None, &["--dry-run", "--accept-guessed"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(guessed)"),
        "{}",
        text(&out.stdout)
    );

    // A database named on its own needs no confirmation, and its covers
    // are still found next to its sqlite/ folder.
    let out = import(&config, None, &["--dry-run", "--db", db.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let summary = text(&out.stdout);
    assert!(
        summary.contains(&format!("old database: {} (given)", db.display())),
        "{summary}"
    );
    assert!(summary.contains("covers (default)"), "{summary}");

    set_writable(&source.path, true);
    assert_eq!(tree(&source.path), before, "the source must not change");
}

#[tokio::test]
async fn refuses_a_target_inside_the_real_source() {
    let source = Source::new(SCHEMA_0617).await;
    let sqlite = source.path.join("sqlite");
    let elsewhere = tempfile::tempdir().unwrap();
    let link = elsewhere.path().join("db.sqlite3");
    std::os::unix::fs::symlink(sqlite.join("db.sqlite3"), &link).unwrap();

    // Through a symlink, the folders checked are the real database's.
    let inside = source.path.join("new");
    let out = import(&link, Some(&inside), &["--root", "Old=a"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("overlaps"),
        "{}",
        text(&out.stderr)
    );
    assert!(!inside.exists());

    // A bare relative file name still has a folder.
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .current_dir(&sqlite)
        .env_remove("KIKOERU_DATA_DIR")
        .args([
            "--data-dir",
            "new",
            "import-legacy",
            "--source",
            "db.sqlite3",
        ])
        .args(["--root", "Old=a"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("overlaps"),
        "{}",
        text(&out.stderr)
    );
    assert!(!sqlite.join("new").exists());
}

#[tokio::test]
async fn finds_covers_next_to_a_database_named_by_a_bare_file_name() {
    let source = Source::new(SCHEMA_0617).await;
    let sqlite = source.path.join("sqlite");
    for args in [
        &["--source", "db.sqlite3"][..],
        &["--source", "..", "--db", "db.sqlite3"][..],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
            .current_dir(&sqlite)
            .env_remove("KIKOERU_DATA_DIR")
            .arg("import-legacy")
            .args(args)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", text(&out.stderr));
        let covers = source.path.join("covers");
        assert!(
            text(&out.stdout).contains(&format!("old covers: {} (default)", covers.display())),
            "{args:?}: {}",
            text(&out.stdout)
        );
    }
}

#[tokio::test]
async fn reads_library_folders_but_no_secrets_from_the_old_config() {
    let source = Source::new(SCHEMA_0617).await;
    let voice = source.path.join("../voice");
    fs::create_dir_all(&voice).unwrap();
    fs::create_dir_all(source.path.join("config")).unwrap();
    fs::write(
        source.path.join("config/config.json"),
        r#"{"version": "0.6.17", "jwtsecret": "JWT-SECRET-VALUE", "md5secret": "MD5-SECRET-VALUE",
            "dbUseDefaultPath": true, "databaseFolderDir": "/usr/src/kikoeru/sqlite",
            "coverFolderDir": "/usr/src/kikoeru/covers",
            "rootFolders": [{"name": "Old", "path": "/usr/src/kikoeru/VoiceWork"},
                            {"name": "Cloud", "type": "openlist", "url": "http://ol:5244",
                             "basePath": "/115", "username": "u", "password": "OL-PASSWORD"}]}"#,
    )
    .unwrap();
    let voice = fs::canonicalize(&voice).unwrap();
    let map = format!("/usr/src/kikoeru/VoiceWork={}", voice.display());
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");
    let roots_file = out_dir.path().join("roots.toml");
    let out = import(
        &source.path,
        Some(&target),
        &[
            "--root",
            "Old=voice",
            "--map-path",
            &map,
            "--write-config",
            roots_file.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let summary = text(&out.stdout);
    assert!(summary.contains("(default, 0.6.17)"), "{summary}");
    assert!(
        summary.contains(&format!(
            "old library Old: local {} (config.json rootFolders)",
            voice.display()
        )),
        "{summary}"
    );
    assert!(summary.contains("old library Cloud: openlist"), "{summary}");
    let roots = fs::read_to_string(&roots_file).unwrap();
    assert!(
        roots.contains(&format!("path = \"{}\"", voice.display())),
        "{roots}"
    );
    // Everything the import wrote or printed, checked for the old secrets.
    let report = fs::read_to_string(target.join("import-report.json")).unwrap();
    for written in [&summary, &text(&out.stderr), &roots, &report] {
        for secret in ["JWT-SECRET-VALUE", "MD5-SECRET-VALUE", "OL-PASSWORD"] {
            assert!(!written.contains(secret), "{secret} leaked");
        }
    }
}

#[tokio::test]
async fn a_discovered_installation_uses_its_configured_database_and_roots() {
    let source = Source::new(&SCHEMA_0617.replace("'marked'", "'listened'")).await;
    let current = Source::new(SCHEMA_0617).await;
    let active = source.path.join("active/db.sqlite3");
    fs::create_dir_all(active.parent().unwrap()).unwrap();
    fs::copy(current.path.join("sqlite/db.sqlite3"), &active).unwrap();
    fs::create_dir_all(source.path.join("config")).unwrap();
    fs::write(
        source.path.join("config/config.json"),
        r#"{"dbUseDefaultPath": false, "databaseFolderDir": "./active",
            "coverUseDefaultPath": true,
            "rootFolders": [{"name": "Old", "path": "./voice"}]}"#,
    )
    .unwrap();
    let voice = source.path.join("voice");
    for work in ["RJ123456", "RJ234567"] {
        fs::create_dir_all(voice.join(work)).unwrap();
    }
    let parent = source.path.parent().unwrap();
    let out = import(parent, None, &["--dry-run"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("--accept-guessed"),
        "{}",
        text(&out.stderr)
    );

    let out_dir = tempfile::tempdir().unwrap();
    let mut configs = Vec::new();
    for (name, pick, origin) in [
        ("direct", source.path.as_path(), "config:databaseFolderDir"),
        ("parent", parent, "guessed"),
    ] {
        let target = out_dir.path().join(name);
        let roots_file = out_dir.path().join(format!("{name}.toml"));
        let out = import(
            pick,
            Some(&target),
            &[
                "--accept-guessed",
                "--root",
                "Old=voice",
                "--write-config",
                roots_file.to_str().unwrap(),
            ],
        );
        assert!(out.status.success(), "{}", text(&out.stderr));
        let report: Value =
            serde_json::from_slice(&fs::read(target.join("import-report.json")).unwrap()).unwrap();
        assert_eq!(report["written"]["favorites"], 1, "{name}: {report}");
        assert_eq!(report["source"]["db"]["path"], active.to_str().unwrap());
        assert_eq!(report["source"]["db"]["origin"], origin);
        assert_eq!(
            report["source"]["covers"]["path"],
            source.path.join("covers").to_str().unwrap()
        );
        let roots = fs::read_to_string(roots_file).unwrap();
        assert!(
            roots.contains(&format!("\npath = \"{}\"", voice.display())),
            "{name}: {roots}"
        );
        configs.push(roots);
    }
    assert_eq!(configs[0], configs[1]);
}

#[tokio::test]
async fn a_signal_during_the_import_leaves_no_partial_target() {
    let source = Source::new(SCHEMA_0617).await;
    // Enough works for the write to pass several cancel checks.
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(source.path.join("sqlite/db.sqlite3")))
        .await
        .unwrap();
    sqlx::raw_sql(
        "WITH RECURSIVE n(i) AS (SELECT 300000 UNION ALL SELECT i + 1 FROM n WHERE i < 304000)
         INSERT INTO t_work (id, root_folder, dir, title, circle_id, lyric_status)
         SELECT i, 'Old', 'RJ' || i, 'w', 11, '' FROM n;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        // Over 20 checks of 100 ms: the write cannot end before the signal
        // lands, however slow the machine is.
        .env("KIKOERU_TEST_IMPORT_PAUSE_MS", "100")
        .arg("--data-dir")
        .arg(&target)
        .args(["import-legacy", "--source"])
        .arg(&source.path)
        .args(["--root", "Old=a"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // The lock appears when the import starts writing. The wait is for that
    // condition, not a time: under a loaded flake check the copy and read
    // before it can take long, and only an exited child ends the wait.
    let lock = target.join(".import-lock");
    while !lock.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let out = child.wait_with_output().unwrap();
            panic!(
                "the import ended ({status}) before it started writing: {}",
                text(&out.stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let out = child.wait_with_output().unwrap();
    assert!(
        !out.status.success(),
        "the import finished before the signal"
    );
    assert!(
        text(&out.stderr).contains("cancelled"),
        "{}",
        text(&out.stderr)
    );
    assert!(!target.exists(), "the target the import created is gone");
}

#[tokio::test]
async fn a_small_import_cancelled_before_writing_leaves_nothing() {
    use clap::Parser;
    let source = Source::new(SCHEMA_0617).await;
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");
    let cli = neo_server::cli::Cli::parse_from([
        "kikoeru-neo".as_ref(),
        "import-legacy".as_ref(),
        "--source".as_ref(),
        source.path.as_os_str(),
        "--root".as_ref(),
        "Old=a".as_ref(),
    ]);
    let neo_server::cli::Command::ImportLegacy(args) = cli.command else {
        unreachable!()
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let db = neo_db::Location::Sqlite(target.join("neo.sqlite3"));
    let err = neo_server::import::run(&args, Some((&target, &db)), &cancel, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("cancelled"), "{err:#}");
    assert!(!target.exists());
}

#[tokio::test]
async fn a_refusing_dry_run_exits_2_and_json_carries_the_refusal() {
    let source = Source::new(SCHEMA_0617).await;
    // Two works in one directory of one root: the import would refuse.
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(source.path.join("sqlite/db.sqlite3")))
        .await
        .unwrap();
    sqlx::raw_sql("UPDATE t_work SET dir = 'RJ123456'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let text_out = import(&source.path, None, &["--dry-run", "--root", "Old=a"]);
    assert_eq!(
        text_out.status.code(),
        Some(2),
        "{}",
        text(&text_out.stderr)
    );
    assert!(text(&text_out.stdout).contains("would refuse to run"));

    let json_out = import(
        &source.path,
        None,
        &["--dry-run", "--json", "--root", "Old=a"],
    );
    assert_eq!(json_out.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(value["report"]["tables"]["t_work"]["source"], 2);
    let refusal = value["refusal"].as_array().unwrap();
    assert!(
        refusal[0].as_str().unwrap().contains("share the directory"),
        "{value}"
    );

    // A clean dry run exits 0 with a null refusal.
    let fine = Source::new(SCHEMA_0617).await;
    let ok = import(&fine.path, None, &["--dry-run", "--json"]);
    assert_eq!(ok.status.code(), Some(0), "{}", text(&ok.stderr));
    let value: Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert!(value["refusal"].is_null());
}
