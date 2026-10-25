//! `import-legacy --merge`: an old library's user data merged into a library
//! that already exists. Every row is made up.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

/// The 0.6.14 schema with what the merge reads: two users, a favorite, two
/// reviews and two play histories, for one work the new library has and
/// one it has not scanned.
const OLD: &str = include_str!("fixtures/legacy_0614.sql");

struct Dirs {
    _dir: tempfile::TempDir,
    old: PathBuf,
    library: PathBuf,
}

async fn setup() -> Dirs {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old");
    fs::create_dir_all(old.join("sqlite")).unwrap();
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

    // A library with its own administrator and one scanned work.
    let library = dir.path().join("library");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .arg("--data-dir")
        .arg(&library)
        .args(["user", "create", "--admin", "admin", "--password-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a long enough password\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    let db = open(&library, false).await;
    sqlx::query(
        "INSERT INTO works (code, title, metadata_src, created_at, updated_at)
         VALUES ('RJ123456', 'scanned', 'none', 0, 0)",
    )
    .execute(&db)
    .await
    .unwrap();
    db.close().await;
    Dirs {
        _dir: dir,
        old,
        library,
    }
}

async fn open(library: &Path, read_only: bool) -> SqlitePool {
    SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(library.join("neo.sqlite3"))
                .read_only(read_only),
        )
        .await
        .unwrap()
}

fn import(dirs: &Dirs, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .env_remove("RUST_LOG")
        .arg("--data-dir")
        .arg(&dirs.library)
        .arg("import-legacy")
        .arg("--source")
        .arg(&dirs.old)
        .args(extra)
        .output()
        .unwrap()
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

async fn count(db: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
}

#[tokio::test]
async fn merges_user_data_and_leaves_the_rest_waiting() {
    let dirs = setup().await;
    let out = import(&dirs, &["--merge", "--user", "guest=friend"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let summary = text(&out.stdout);
    assert!(
        summary.contains("merged user data for 2 works"),
        "{summary}"
    );
    assert!(summary.contains("works waiting for a scan: 1"), "{summary}");
    assert!(
        summary.contains("created without a password: friend"),
        "{summary}"
    );

    let db = open(&dirs.library, true).await;
    // The library's own works and accounts stay. Nothing old is imported
    // as a work.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM works").await, 1);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM users WHERE is_admin = 1").await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM users WHERE name = 'friend' AND password_hash IS NULL"
        )
        .await,
        1
    );
    // The scanned work gets the old admin's favorite and position at once,
    // on the library's admin of the same name.
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM favorites f JOIN users u ON u.id = f.user_id
             WHERE u.name = 'admin' AND f.work_id = 1"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT position_ms FROM play_progress WHERE work_id = 1"
        )
        .await,
        30_000
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM import_pending WHERE state = 'pending'"
        )
        .await,
        1
    );
    // The other work's data waits for a scan, for both users.
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM legacy_user_state WHERE code = 'RJ234567'"
        )
        .await,
        2
    );
    db.close().await;

    // A second merge of the same library changes nothing that attached.
    let again = import(&dirs, &["--merge", "--user", "guest=friend"]);
    assert!(again.status.success(), "{}", text(&again.stderr));
    let db = open(&dirs.library, true).await;
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM legacy_user_state").await,
        2
    );
    db.close().await;
}

#[tokio::test]
async fn refuses_a_missing_library_and_wrong_arguments() {
    let dirs = setup().await;
    let dry = import(&dirs, &["--merge", "--dry-run"]);
    assert!(dry.status.success(), "{}", text(&dry.stderr));
    assert!(text(&dry.stdout).contains("would merge user data for 2 works"));

    let out = import(&dirs, &["--merge", "--user", "nobody=x"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("no user \"nobody\""),
        "{}",
        text(&out.stderr)
    );

    let out = import(&dirs, &["--user", "guest=friend"]);
    assert!(!out.status.success());

    fs::remove_dir_all(&dirs.library).unwrap();
    let out = import(&dirs, &["--merge"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("no library to merge into"),
        "{}",
        text(&out.stderr)
    );
    assert!(!dirs.library.join("neo.sqlite3").exists());
}

#[tokio::test]
async fn never_writes_to_a_library_that_is_the_source() {
    let dirs = setup().await;
    let before = fs::read(dirs.old.join("sqlite/db.sqlite3")).unwrap();
    // A library whose database is a link to the old one.
    let alias = dirs.library.parent().unwrap().join("alias");
    fs::create_dir(&alias).unwrap();
    std::os::unix::fs::symlink(
        dirs.old.join("sqlite/db.sqlite3"),
        alias.join("neo.sqlite3"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .arg("--data-dir")
        .arg(&alias)
        .args(["import-legacy", "--merge", "--source"])
        .arg(&dirs.old)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("overlaps"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(
        fs::read(dirs.old.join("sqlite/db.sqlite3")).unwrap(),
        before
    );

    // A hard link has no path in common with the source.
    let hard = dirs.library.parent().unwrap().join("hard");
    fs::create_dir(&hard).unwrap();
    fs::hard_link(dirs.old.join("sqlite/db.sqlite3"), hard.join("neo.sqlite3")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .arg("--data-dir")
        .arg(&hard)
        .args(["import-legacy", "--merge", "--source"])
        .arg(&dirs.old)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("is the old database itself"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(
        fs::read(dirs.old.join("sqlite/db.sqlite3")).unwrap(),
        before
    );
}

#[tokio::test]
async fn two_old_users_merge_into_one_account_without_losing_data() {
    let dirs = setup().await;
    let out = import(&dirs, &["--merge", "--user", "guest=admin"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // The plan's report comes first, so skipped rows are not silent.
    assert!(
        text(&out.stdout).contains("\nissues: "),
        "{}",
        text(&out.stdout)
    );
    let db = open(&dirs.library, true).await;
    assert_eq!(count(&db, "SELECT COUNT(*) FROM users").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);
    // The old admin's position and the guest's review for RJ234567 both
    // wait under the one account. The review keeps its time for when the
    // scan moves it into reviews.
    let waiting: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT rating, review_at, position_ms FROM legacy_user_state WHERE code = 'RJ234567'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(waiting, (Some(4), Some(1_704_067_200_000), Some(5000)));
    db.close().await;
}

#[tokio::test]
async fn a_blank_review_does_not_push_out_another_users_rating() {
    let dirs = setup().await;
    // Old kikoeru saves an empty text when a review is edited in full, so
    // the first user's rows say nothing although their fields are set.
    let old = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(dirs.old.join("sqlite/db.sqlite3")))
        .await
        .unwrap();
    sqlx::raw_sql(
        "UPDATE t_review SET review_text = '' WHERE user_name = 'admin';
         INSERT INTO t_review (user_name, work_id, rating, review_text, updated_at) VALUES
           ('admin', 234567, 9, '  ', '2024-01-01 00:00:00'),
           ('guest', 123456, 5, NULL, '2024-01-01 00:00:00');",
    )
    .execute(&old)
    .await
    .unwrap();
    old.close().await;

    let out = import(&dirs, &["--merge", "--user", "guest=admin"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let db = open(&dirs.library, true).await;
    // The scanned work gets the guest's rating at once.
    let scanned: Vec<(Option<i64>, Option<String>)> =
        sqlx::query_as("SELECT rating, text FROM reviews WHERE work_id = 1")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(scanned, [(Some(5), None)]);
    // The work not scanned yet keeps the guest's rating for the scan.
    let waiting: Option<i64> =
        sqlx::query_scalar("SELECT rating FROM legacy_user_state WHERE code = 'RJ234567'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(waiting, Some(4));
    db.close().await;
}

#[tokio::test]
async fn waits_for_a_server_that_is_writing() {
    let dirs = setup().await;
    // Another writer, like the running server, holds the write lock and
    // commits while the merge is starting.
    let db = open(&dirs.library, false).await;
    let mut writer = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("INSERT INTO users (name, is_admin, created_at) VALUES ('busy', 0, 0)")
        .execute(&mut *writer)
        .await
        .unwrap();
    let library = dirs.library.clone();
    let old = dirs.old.clone();
    let merge = std::thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
            .env_remove("KIKOERU_DATA_DIR")
            .arg("--data-dir")
            .arg(&library)
            .args(["import-legacy", "--merge", "--source"])
            .arg(&old)
            .output()
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    writer.commit().await.unwrap();
    db.close().await;
    let out = merge.join().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
}

/// The arguments of `import-legacy --merge` for a library call.
fn merge_args(dirs: &Dirs) -> neo_server::cli::ImportArgs {
    use clap::Parser;
    let cli = neo_server::cli::Cli::parse_from([
        "kikoeru-neo".as_ref(),
        "import-legacy".as_ref(),
        "--merge".as_ref(),
        "--source".as_ref(),
        dirs.old.as_os_str(),
    ]);
    match cli.command {
        neo_server::cli::Command::ImportLegacy(args) => args,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn a_cancelled_merge_leaves_the_library_unchanged() {
    let dirs = setup().await;
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let mut out = Vec::new();
    let db = neo_db::Location::Sqlite(dirs.library.join("neo.sqlite3"));
    let err = neo_server::import::run(
        &merge_args(&dirs),
        Some((&dirs.library, &db)),
        &cancel,
        &mut out,
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("cancelled"), "{err:#}");
    let db = open(&dirs.library, true).await;
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM legacy_user_state").await,
        0
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM users").await, 1);
    db.close().await;
}
