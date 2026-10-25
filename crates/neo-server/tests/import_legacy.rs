//! `import-legacy` against a synthetic old data directory. The DDL is the old
//! kikoeru schema as of its last migration that the live data has
//! (20241110142802_custom_work_meta.js). Every row is made up.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

const OLD_SCHEMA: &str = r#"
CREATE TABLE `knex_migrations` (`id` integer not null primary key autoincrement, `name` varchar(255), `batch` integer, `migration_time` datetime);
CREATE TABLE `t_circle` (`id` integer not null primary key autoincrement, `name` varchar(255) not null);
CREATE TABLE `t_va` (`id` varchar(255), `name` varchar(255) not null, primary key (`id`));
CREATE TABLE `t_tag` (`id` integer not null primary key autoincrement, `name` varchar(255) not null);
CREATE TABLE `t_work` (`id` integer not null primary key autoincrement, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `root_folder` varchar(255) not null, `dir` varchar(255) not null, `title` varchar(255) not null, `circle_id` integer not null, `nsfw` boolean, `release` varchar(255), `dl_count` integer, `price` integer, `review_count` integer, `rate_count` integer, `rate_average_2dp` float, `rate_count_detail` text, `rank` text, `lyric_status` varchar(255) not null, `original_work_id` bigint not null default '0', `memo` json, `is_custom_meta` integer default '0', foreign key(`circle_id`) references `t_circle`(`id`));
CREATE TABLE `r_tag_work` (`tag_id` integer, `work_id` bigint, foreign key(`tag_id`) references `t_tag`(`id`), foreign key(`work_id`) references `t_work`(`id`), primary key (`tag_id`, `work_id`));
CREATE TABLE `r_va_work` (`va_id` varchar(255), `work_id` bigint, foreign key(`va_id`) references `t_va`(`id`) on delete CASCADE on update CASCADE, foreign key(`work_id`) references `t_work`(`id`) on delete CASCADE on update CASCADE, primary key (`va_id`, `work_id`));
CREATE TABLE `t_user` (`name` varchar(255) not null, `password` varchar(255) not null, `group` varchar(255) not null, primary key (`name`));
CREATE TABLE `t_review` (`user_name` varchar(255) not null, `work_id` bigint not null, `rating` integer, `review_text` varchar(255), `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `progress` varchar(255), foreign key(`user_name`) references `t_user`(`name`) on delete CASCADE, foreign key(`work_id`) references `t_work`(`id`) on delete CASCADE, primary key (`user_name`, `work_id`));
CREATE TABLE `t_play_histroy` (`user_name` varchar(255) not null, `work_id` bigint not null, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `state` json not null, foreign key(`user_name`) references `t_user`(`name`) on delete CASCADE, foreign key(`work_id`) references `t_work`(`id`) on delete CASCADE, primary key (`user_name`, `work_id`));
CREATE TABLE `t_translate_task` (`id` integer not null primary key autoincrement, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `work_id` bigint not null default '0', `audio_path` varchar(255) not null default '', `status` integer not null default '0', `worker_name` varchar(255) not null default '', `worker_status` varchar(255) not null default '', `secret` varchar(255) not null default '', foreign key(`work_id`) references `t_work`(`id`));
"#;

// Old work IDs. BJ shares its digits with RJ_SHORT to check that the prefix
// is decoded from the ID and both works survive.
const RJ_SHORT: i64 = 123_456;
const RJ_LONG: i64 = 1_234_567;
const BJ_SAME: i64 = 1_000_000_123_456;
const RJ_CUSTOM: i64 = 200_000;
const RJ_PLAIN: i64 = 300_000;

const MEMO: &str = r#"{"duration":{"mp3/SE/01.mp3":61.5,"mp3/NoSE/01.mp3":61.5,"02.mp3":90},"mtime":{},"isContainLyric":false}"#;

const OLD_DATA: &str = r#"
INSERT INTO knex_migrations (name, batch) VALUES ('20241110142802_custom_work_meta.js', 1);
INSERT INTO t_circle VALUES (11, 'Circle A'), (1000000022, 'Circle B');
INSERT INTO t_va VALUES ('va-1', 'Voice One'), ('va-2', 'Voice Two');
INSERT INTO t_tag VALUES (1, 'tag one'), (2, 'tag two');
INSERT INTO t_user VALUES ('admin', 'oldhash', 'administrator'), ('guest', 'oldhash', 'user');
INSERT INTO t_work (id, created_at, updated_at, root_folder, dir, title, circle_id, nsfw, release, dl_count, price, review_count, rate_count, rate_average_2dp, rate_count_detail, rank, lyric_status, original_work_id, memo, is_custom_meta) VALUES
  (123456, '2024-01-02 03:04:05', '2024-01-03 00:00:00', 'Old A', 'RJ123456', 'Short', 11, 1, '2010-05-06', 10, 770, 1, 5, 4.25, '[{"review_point":5,"count":5,"ratio":100}]', '[{"term":"year","category":"voice","rank":3,"rank_date":"2012"}]', '', 0, :memo, 0),
  (1234567, '2024-01-02 03:04:06', '2024-01-02 03:04:06', 'Old A', 'nested/RJ01234567 title', 'Long', 11, 0, 'not a date', NULL, NULL, 0, 0, 0, '[]', 'null', '', 0, '{}', 0),
  (1000000123456, '2024-01-02 03:04:07', '2024-01-02 03:04:07', 'Old B', 'BJ123456', 'Same digits', 1000000022, 1, '2020-01-01', 1, 1, 0, 0, 0, '[]', 'null', '', 0, '{"duration":{}}', 0),
  (200000, '2024-01-02 03:04:08', '2024-01-02 03:04:08', 'Old B', 'RJ0200000', 'Custom', 11, 1, '2020-01-01', 1, 1, 0, 0, 0, '[]', 'null', '', 0, '{}', 1),
  (300000, '2024-01-02 03:04:09', '2024-01-02 03:04:09', 'Old B', 'RJ300000', 'Plain', 11, 1, '2020-01-01', 1, 1, 0, 0, 0, '[]', 'null', '', 0, :memo, 0);
INSERT INTO r_tag_work VALUES (1, 123456), (2, 123456), (1, 1000000123456);
INSERT INTO r_va_work VALUES ('va-1', 123456), ('va-2', 1234567);
INSERT INTO t_review (user_name, work_id, rating, review_text, created_at, updated_at, progress) VALUES
  ('admin', 123456, NULL, NULL, '2024-02-01 00:00:00', '2024-02-01 00:00:00', 'marked'),
  ('admin', 1234567, 9, NULL, '2024-02-02 00:00:00', '2024-02-02 00:00:00', 'marked'),
  ('admin', 1000000123456, NULL, NULL, '2024-02-03 00:00:00', '2024-02-03 00:00:00', 'listened'),
  ('admin', 300000, NULL, replace(hex(zeroblob(2001)), '00', 'x'), '2024-02-04 00:00:00', '2024-02-04 00:00:00', 'listened'),
  ('guest', 123456, 4, 'nice', '2024-02-05 00:00:00', '2024-02-05 00:00:00', 'replay');
INSERT INTO t_play_histroy (user_name, work_id, created_at, updated_at, state) VALUES
  ('admin', 123456, '2024-03-01 00:00:00', '2024-03-01 00:00:00',
   '{"queue":[{"title":"01.mp3","subtitle":"mp3/NoSE"},{"title":"01.mp3","subtitle":"mp3/SE"}],"index":1,"seconds":12.5}'),
  ('admin', 1000000123456, '2024-03-02 00:00:00', '2024-03-02 00:00:00',
   '{"queue":[{"title":"x.mp3","subtitle":""}],"index":0,"seconds":7}'),
  ('admin', 200000, '2024-03-03 00:00:00', '2024-03-03 00:00:00', '{"queue":'),
  ('guest', 300000, '2024-03-04 00:00:00', '2024-03-04 00:00:00',
   '{"queue":[{"title":"02.mp3"}],"index":5,"seconds":3}');
"#;

/// A synthetic old data directory: database plus covers for every work but
/// RJ_PLAIN.
struct Source {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Source {
    async fn new() -> Self {
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
        sqlx::raw_sql(OLD_SCHEMA).execute(&pool).await.unwrap();
        let data = OLD_DATA.replace(":memo", &format!("'{MEMO}'"));
        sqlx::raw_sql(sqlx::AssertSqlSafe(data))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        // A symlinked cover is not followed, so RJ300000 has no cover.
        std::os::unix::fs::symlink(
            path.join("sqlite/db.sqlite3"),
            path.join("covers/RJ300000_img_main.jpg"),
        )
        .unwrap();
        for code in ["RJ123456", "RJ01234567", "BJ123456", "RJ200000"] {
            fs::write(
                path.join(format!("covers/{code}_img_main.jpg")),
                format!("jpeg {code}"),
            )
            .unwrap();
            fs::write(path.join(format!("covers/{code}_img_sam.jpg")), "small").unwrap();
        }
        Self { _dir: dir, path }
    }

    fn db_bytes(&self) -> Vec<u8> {
        fs::read(self.path.join("sqlite/db.sqlite3")).unwrap()
    }
}

fn import(source: &Path, target: Option<&Path>, extra: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"));
    cmd.env_remove("KIKOERU_DATA_DIR")
        .env_remove("KIKOERU_LISTEN")
        .env_remove("RUST_LOG");
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

const ROOTS: [&str; 4] = ["--root", "Old A=a", "--root", "Old B=b"];

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

async fn work_id(pool: &SqlitePool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM works WHERE code = ?")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn imports_every_mapping_rule() {
    let source = Source::new().await;
    let before = source.db_bytes();
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");

    let out = import(&source.path, Some(&target), &ROOTS);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(source.db_bytes(), before, "the source must not change");
    assert!(!target.join(".import-lock").exists());
    let db = open(&target).await;

    // Works, codes and locations.
    let codes: Vec<String> = sqlx::query_scalar("SELECT code FROM works ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(
        codes,
        ["RJ123456", "RJ200000", "RJ300000", "RJ01234567", "BJ123456"]
    );
    let row = sqlx::query(
        "SELECT title, circle_id, release_date, nsfw, price, rate_average, metadata_src,
                cover_src, extra, created_at, updated_at FROM works WHERE code = 'RJ123456'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("title"), "Short");
    assert_eq!(row.get::<i64, _>("circle_id"), 11);
    assert_eq!(row.get::<String, _>("release_date"), "2010-05-06");
    assert!(row.get::<bool, _>("nsfw"));
    assert_eq!(row.get::<i64, _>("price"), 770);
    assert_eq!(row.get::<f64, _>("rate_average"), 4.25);
    assert_eq!(row.get::<String, _>("metadata_src"), "dlsite");
    assert_eq!(row.get::<String, _>("cover_src"), "imported");
    // 2024-01-02 03:04:05 UTC
    assert_eq!(row.get::<i64, _>("created_at"), 1_704_164_645_000);
    assert_eq!(row.get::<i64, _>("updated_at"), 1_704_240_000_000);
    let extra: Value = serde_json::from_str(&row.get::<String, _>("extra")).unwrap();
    assert_eq!(extra["rank"][0]["rank"], 3);
    assert_eq!(extra["rate_count_detail"][0]["count"], 5);
    assert_eq!(extra["legacy_durations"]["mp3/SE/01.mp3"], 61.5);

    let long: (Option<String>, String) =
        sqlx::query_as("SELECT release_date, extra FROM works WHERE code = 'RJ01234567'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        long,
        (None, "{}".to_owned()),
        "bad date and empty JSON dropped"
    );
    let custom: String =
        sqlx::query_scalar("SELECT metadata_src FROM works WHERE code = 'RJ200000'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(custom, "manual");
    let plain: String = sqlx::query_scalar("SELECT cover_src FROM works WHERE code = 'RJ300000'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(plain, "none");

    let locations: Vec<(String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT l.root_id, l.rel_path, l.display_path FROM work_locations l
         JOIN works w ON w.id = l.work_id ORDER BY w.id",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(locations[3].0, "a");
    assert_eq!(locations[3].1, b"nested/RJ01234567 title");
    assert_eq!(locations[3].2, "nested/RJ01234567 title");
    assert_eq!(locations[4].0, "b");
    let roots: Vec<(String, String)> =
        sqlx::query_as("SELECT id, display_name FROM roots ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        roots,
        [("a".into(), "Old A".into()), ("b".into(), "Old B".into())]
    );

    // Covers: only main, named by new ID, private.
    let short = work_id(&db, "RJ123456").await;
    let cover = target.join(format!("covers/{short}.jpg"));
    assert_eq!(fs::read_to_string(&cover).unwrap(), "jpeg RJ123456");
    assert_eq!(
        fs::metadata(&cover).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(target.join("covers")).unwrap().count(), 4);

    // Dictionaries and relations.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM circles").await, 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM vas").await, 2);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM tag_aliases a JOIN tags t ON t.id = a.tag_id AND t.name = a.alias"
        )
        .await,
        2
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM work_tags").await, 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM work_vas").await, 2);

    // Users get no password and a random token version.
    let users: Vec<(String, Option<String>, bool)> =
        sqlx::query_as("SELECT name, password_hash, is_admin FROM users ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        users,
        [("admin".into(), None, true), ("guest".into(), None, false)]
    );

    // Favorites come from marked reviews only.
    let favorites: Vec<String> = sqlx::query_scalar(
        "SELECT w.code FROM favorites f JOIN works w ON w.id = f.work_id ORDER BY w.code",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(favorites, ["RJ01234567", "RJ123456"]);

    // play_progress: union of play history and listened reviews.
    let progress: Vec<(String, String, i64, Option<i64>, i64)> = sqlx::query_as(
        "SELECT u.name, w.code, p.position_ms, p.finished_at, p.updated_at
         FROM play_progress p JOIN works w ON w.id = p.work_id JOIN users u ON u.id = p.user_id
         WHERE p.track_id IS NULL ORDER BY u.name, w.code",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    let day = |d: i64| 1_704_067_200_000 + d * 86_400_000; // 2024-01-01 + d days
    assert_eq!(
        progress,
        [
            // History and a listened review: both the position and the mark.
            (
                "admin".into(),
                "BJ123456".into(),
                7_000,
                Some(day(33)),
                day(61)
            ),
            ("admin".into(), "RJ123456".into(), 12_500, None, day(60)),
            // Unparsable state: kept at position 0.
            ("admin".into(), "RJ200000".into(), 0, None, day(62)),
            // Listened without history.
            ("admin".into(), "RJ300000".into(), 0, Some(day(34)), day(34)),
            // Index outside the queue: position kept.
            ("guest".into(), "RJ300000".into(), 3_000, None, day(63)),
        ]
    );

    // legacy_reviews keeps every review as it was.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM legacy_reviews").await, 5);
    let replay: (Option<i64>, Option<String>, String) = sqlx::query_as(
        "SELECT rating, review_text, progress FROM legacy_reviews r JOIN users u ON u.id = r.user_id
         WHERE u.name = 'guest'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(replay, (Some(4), Some("nice".into()), "replay".into()));
    // reviews gets what the pages show: valid ratings and texts with the old
    // time. The out-of-range rating is dropped and the long text cut.
    // User, work code, rating, text length and update time.
    type Shown = (String, String, Option<i64>, Option<i64>, Option<i64>);
    let shown: Vec<Shown> = sqlx::query_as(
        "SELECT u.name, w.code, r.rating, length(r.text), r.updated_at FROM reviews r
         JOIN users u ON u.id = r.user_id JOIN works w ON w.id = r.work_id ORDER BY u.name",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        shown,
        [
            (
                "admin".into(),
                "RJ300000".into(),
                None,
                Some(2000),
                Some(day(34))
            ),
            (
                "guest".into(),
                "RJ123456".into(),
                Some(4),
                Some(4),
                Some(day(35))
            ),
        ]
    );

    // import_pending keeps what later matching needs.
    let pending: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT w.code, p.state, p.source, p.durations FROM import_pending p
         JOIN works w ON w.id = p.work_id ORDER BY p.user_id, w.code",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    let states: Vec<(&str, &str)> = pending
        .iter()
        .map(|p| (p.0.as_str(), p.1.as_str()))
        .collect();
    assert_eq!(
        states,
        [
            ("BJ123456", "pending"),
            ("RJ123456", "pending"),
            ("RJ200000", "unmatched"),
            ("RJ300000", "unmatched")
        ]
    );
    let source_json: Value = serde_json::from_str(&pending[1].2).unwrap();
    assert_eq!(source_json["item"]["subtitle"], "mp3/SE");
    assert_eq!(source_json["seconds"], 12.5);
    assert_eq!(source_json["legacy_row"]["work_id"], RJ_SHORT);
    let durations: Value = serde_json::from_str(pending[1].3.as_deref().unwrap()).unwrap();
    assert_eq!(durations["02.mp3"], 90);
    let bad: Value = serde_json::from_str(&pending[2].2).unwrap();
    assert_eq!(bad["raw"], r#"{"queue":"#);

    // The report holds the ID map and every issue.
    let report_path = target.join("import-report.json");
    assert_eq!(
        fs::metadata(&report_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    let ids = report["work_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 5);
    assert!(ids.contains(&serde_json::json!([
        BJ_SAME,
        work_id(&db, "BJ123456").await
    ])));
    assert_eq!(report["written"]["play_progress"], 5);
    assert_eq!(report["written"]["covers"], 4);
    assert_eq!(report["import_pending"]["unmatched"], 2);
    let issues = report["issues"].to_string();
    for needle in [
        "no cover RJ300000_img_main.jpg",
        "release \\\"not a date\\\"",
        "state is not a JSON object",
        "index is outside the queue",
        "rating is not an integer from 1 to 5",
        "longer than 2000 characters",
    ] {
        assert!(issues.contains(needle), "{needle} missing from {issues}");
    }
    db.close().await;
}

#[tokio::test]
async fn dry_run_reports_the_same_numbers_and_writes_nothing() {
    let source = Source::new().await;
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");

    let mut args = vec!["--dry-run"];
    args.extend(ROOTS);
    let dry = import(&source.path, Some(&target), &args);
    assert!(dry.status.success(), "{}", text(&dry.stderr));
    assert!(!target.exists(), "a dry run writes nothing");

    let real = import(&source.path, Some(&target), &ROOTS);
    assert!(real.status.success(), "{}", text(&real.stderr));
    // The import prints the dry run's summary plus where it wrote.
    assert!(
        text(&real.stdout).starts_with(&text(&dry.stdout)),
        "dry run:\n{}\nimport:\n{}",
        text(&dry.stdout),
        text(&real.stdout)
    );
    let summary = text(&dry.stdout);
    for line in [
        "codes by prefix/digits: BJ/6=1, RJ/6=3, RJ/8=1",
        "directories with a nonstandard code width: 1",
        "reviewed works with play history: 2",
    ] {
        assert!(summary.contains(line), "{line} missing from {summary}");
    }

    // Without --root the dry run lists the root folders that need one.
    let bare = import(&source.path, None, &["--dry-run"]);
    assert!(bare.status.success(), "{}", text(&bare.stderr));
    assert!(text(&bare.stdout).contains("root_folder: Old A=2, Old B=3"));
}

#[tokio::test]
async fn refuses_unsafe_targets_and_incomplete_root_maps() {
    let source = Source::new().await;
    let out_dir = tempfile::tempdir().unwrap();

    let busy = out_dir.path().join("busy");
    fs::create_dir(&busy).unwrap();
    fs::write(busy.join("neo.sqlite3"), "").unwrap();
    let out = import(&source.path, Some(&busy), &ROOTS);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("not empty"),
        "{}",
        text(&out.stderr)
    );

    let inside = source.path.join("new");
    let out = import(&source.path, Some(&inside), &ROOTS);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("overlaps"),
        "{}",
        text(&out.stderr)
    );
    assert!(!inside.exists());

    // A dangling symlink is neither followed nor removed.
    let link = out_dir.path().join("link");
    std::os::unix::fs::symlink(out_dir.path().join("nowhere"), &link).unwrap();
    let out = import(&source.path, Some(&link), &ROOTS);
    assert!(!out.status.success());
    assert!(fs::symlink_metadata(&link).is_ok());
    assert!(!out_dir.path().join("nowhere").exists());

    let target = out_dir.path().join("new");
    let out = import(&source.path, Some(&target), &["--root", "Old A=a"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains(r#"no --root mapping for root_folder "Old B""#),
        "{}",
        text(&out.stderr)
    );
    assert!(!target.exists());

    let out = import(&source.path, Some(&target), &[]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("--root OLD=NEW"));

    let out = import(&source.path, Some(&target), &["--root", "Old A=not ok"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("ASCII letters"));

    // A database with uncheckpointed WAL data is refused by default,
    // because reading the main file alone would silently miss those changes.
    let wal = source.path.join("sqlite/db.sqlite3-wal");
    fs::write(&wal, "pending frames").unwrap();
    let out = import(&source.path, None, &["--dry-run"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("--recover-journal"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(fs::read(&wal).unwrap(), b"pending frames");
    fs::remove_file(&wal).unwrap();

    // Two old roots may become one new root.
    let out = import(
        &source.path,
        Some(&target),
        &["--root", "Old A=same", "--root", "Old B=same"],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
}

#[tokio::test]
async fn imports_accounts_from_a_library_without_works() {
    let source = Source::new().await;
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(source.path.join("sqlite/db.sqlite3")))
        .await
        .unwrap();
    sqlx::raw_sql(
        "DELETE FROM t_play_histroy; DELETE FROM t_review; DELETE FROM r_tag_work;
         DELETE FROM r_va_work; DELETE FROM t_work;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");

    let out = import(&source.path, Some(&target), &[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let db = open(&target).await;
    assert_eq!(count(&db, "SELECT COUNT(*) FROM users").await, 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM works").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM roots").await, 0);
    db.close().await;
}

#[tokio::test]
async fn a_failed_import_leaves_the_target_empty() {
    let source = Source::new().await;
    // An unreadable cover fails the import after the database is written.
    let cover = source.path.join("covers/RJ123456_img_main.jpg");
    fs::set_permissions(&cover, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&cover).is_ok() {
        // Running as root: permissions cannot make the file unreadable.
        return;
    }
    let out_dir = tempfile::tempdir().unwrap();

    let fresh = out_dir.path().join("fresh");
    let out = import(&source.path, Some(&fresh), &ROOTS);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cleaned up"),
        "{}",
        text(&out.stderr)
    );
    assert!(!fresh.exists());

    let empty = out_dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let out = import(&source.path, Some(&empty), &ROOTS);
    assert!(!out.status.success());
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
}

#[test]
fn constants_match_the_fixture() {
    // Keeps the named IDs above in step with OLD_DATA.
    for id in [RJ_SHORT, RJ_LONG, BJ_SAME, RJ_CUSTOM, RJ_PLAIN] {
        assert!(OLD_DATA.contains(&format!("({id},")), "{id}");
    }
}
