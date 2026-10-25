//! Runs the real binary for behaviour that only shows at the process level:
//! exit codes, stdin passwords and startup failures.

use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use neo_db::{Db, users};

fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"));
    cmd.env_remove("KIKOERU_DATA_DIR")
        .env_remove("KIKOERU_LISTEN")
        .env_remove("RUST_LOG");
    cmd
}

fn run(data_dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = bin()
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

async fn user(data_dir: &Path, name: &str) -> Option<users::UserRecord> {
    let db = Db::open_sqlite(&data_dir.join("neo.sqlite3"))
        .await
        .unwrap();
    let user = users::find_by_name(&db, name).await.unwrap();
    db.close().await;
    user
}

#[tokio::test]
async fn user_commands() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let create = |name: &str, admin: bool, password: &str| {
        let mut args = vec!["user", "create", name, "--password-stdin"];
        if admin {
            args.push("--admin");
        }
        run(d, &args, &format!("{password}\n"))
    };

    let out = create("alice", true, "alice password");
    assert!(out.status.success(), "{}", stderr(&out));
    let alice = user(d, "alice").await.unwrap();
    assert!(alice.is_admin);
    assert!(neo_server::password::verify(
        alice.password_hash.as_deref(),
        "alice password"
    ));
    assert!(
        !d.join("secrets/jwt.key").exists(),
        "only serve creates the key"
    );

    let out = create("alice", false, "another password");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    let out = create("short", false, "1234567");
    assert!(!out.status.success());
    assert!(user(d, "short").await.is_none());

    let out = run(
        d,
        &["user", "set-password", "alice", "--password-stdin"],
        "new password\r\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let after = user(d, "alice").await.unwrap();
    assert_eq!(after.token_version, alice.token_version + 1);
    assert!(neo_server::password::verify(
        after.password_hash.as_deref(),
        "new password"
    ));
    let out = run(
        d,
        &["user", "set-password", "nobody", "--password-stdin"],
        "new password\n",
    );
    assert!(!out.status.success());

    let out = run(d, &["user", "delete", "alice"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("last admin"), "{}", stderr(&out));
    assert!(user(d, "alice").await.is_some());

    assert!(create("bob", false, "bob password").status.success());
    assert!(run(d, &["user", "delete", "bob"], "").status.success());
    assert!(user(d, "bob").await.is_none());
    assert!(!run(d, &["user", "delete", "bob"], "").status.success());
}

#[test]
fn data_dir_is_required() {
    let out = bin().args(["user", "delete", "x"]).output().unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("KIKOERU_DATA_DIR"),
        "{}",
        stderr(&out)
    );
}

#[tokio::test]
async fn env_data_dir_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = bin()
        .env("KIKOERU_DATA_DIR", dir.path())
        .args(["user", "create", "--admin", "alice", "--password-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"alice password\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(user(dir.path(), "alice").await.is_some());
}

fn write_config(dir: &Path, text: &str) {
    std::fs::write(dir.join("config.toml"), text).unwrap();
}

#[test]
fn invalid_config_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    write_config(
        dir.path(),
        "listen = \"127.0.0.1:0\"\n[media]\nheavy_jobs = -1\n",
    );
    let out = run(dir.path(), &["serve"], "");
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("media.heavy_jobs") && err.contains("-1"),
        "{err}"
    );
    assert!(!dir.path().join("neo.sqlite3").exists());
}

#[test]
fn invalid_listen_env_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin()
        .arg("--data-dir")
        .arg(dir.path())
        .arg("serve")
        .env("KIKOERU_LISTEN", "0.0.0.0")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("KIKOERU_LISTEN") && err.contains("\"0.0.0.0\""),
        "{err}"
    );
    assert!(!dir.path().join("neo.sqlite3").exists());
}

#[test]
fn missing_explicit_config_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        dir.path(),
        &["serve", "--config", "/nonexistent/config.toml"],
        "",
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("/nonexistent/config.toml"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn corrupt_database_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "listen = \"127.0.0.1:0\"\n");
    std::fs::write(
        dir.path().join("neo.sqlite3"),
        b"definitely not a sqlite database file, padded to look like one",
    )
    .unwrap();
    let out = run(dir.path(), &["serve"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("database"), "{}", stderr(&out));
}

#[tokio::test]
async fn failed_migration_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "listen = \"127.0.0.1:0\"\n");
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    // A migration this build does not know about, as if a newer build had
    // already upgraded the schema.
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (99999999, 'from the future', 1, x'00', 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    db.close().await;
    let out = run(dir.path(), &["serve"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("migration"), "{}", stderr(&out));
}

#[test]
fn backup_waits_for_a_cover_update() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("data");
    let out = run(
        &d,
        &["user", "create", "alice", "--admin", "--password-stdin"],
        "alice password\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let updating = neo_scanner::metadata::lock_covers(&d.join("covers")).unwrap();
    let dest = dir.path().join("backup");
    let mut backup = bin()
        .arg("--data-dir")
        .arg(&d)
        .args(["backup", dest.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!dest.join("neo.sqlite3").exists());
    drop(updating);
    assert!(backup.wait().unwrap().success());
    assert!(dest.join("neo.sqlite3").exists());
}

#[tokio::test]
async fn backup_copies_state_while_the_database_is_open() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("data");
    let out = run(
        &d,
        &["user", "create", "alice", "--admin", "--password-stdin"],
        "alice password\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    std::fs::write(d.join("covers/1.jpg"), b"cover").unwrap();
    std::fs::write(d.join("cache/thumbs/1.webp"), b"thumb").unwrap();
    std::os::unix::fs::symlink("/etc/hostname", d.join("lyrics/link.lrc")).unwrap();
    // A second user that is still only in the -wal file while the backup runs.
    let db = Db::open_sqlite(&d.join("neo.sqlite3")).await.unwrap();
    sqlx::query("PRAGMA wal_autocheckpoint = 0")
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (name, password_hash, is_admin, created_at) VALUES ('bob', 'x', 0, 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();

    let dest = dir.path().join("backup");
    let out = run(&d, &["backup", dest.to_str().unwrap()], "");
    assert!(out.status.success(), "{}", stderr(&out));
    db.close().await;

    assert_eq!(std::fs::read(dest.join("covers/1.jpg")).unwrap(), b"cover");
    assert!(
        !dest.join("cache").exists(),
        "the cache is rebuilt, not backed up"
    );
    assert!(
        !dest.join("lyrics/link.lrc").exists(),
        "symlinks are skipped"
    );
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dest), 0o700);
    assert_eq!(mode(&dest.join("neo.sqlite3")), 0o600);
    assert!(user(&dest, "alice").await.is_some());
    assert!(
        user(&dest, "bob").await.is_some(),
        "the -wal contents are in the copy"
    );

    let again = run(&d, &["backup", dest.to_str().unwrap()], "");
    assert!(
        !again.status.success(),
        "an existing destination is refused"
    );
    let missing = run(
        &dir.path().join("typo"),
        &["backup", dir.path().join("b2").to_str().unwrap()],
        "",
    );
    assert!(!missing.status.success());
    assert!(
        !dir.path().join("typo").exists(),
        "no data directory is created"
    );

    // Inside the data directory, also through a symlinked parent, the copy
    // would read its own output.
    std::os::unix::fs::symlink(d.join("covers"), dir.path().join("alias")).unwrap();
    for inside in [d.join("covers/backup"), dir.path().join("alias/backup")] {
        let out = run(&d, &["backup", inside.to_str().unwrap()], "");
        assert!(!out.status.success(), "{}", inside.display());
        assert!(
            stderr(&out).contains("inside the data directory"),
            "{}",
            stderr(&out)
        );
    }
    assert!(!d.join("covers/backup").exists());
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn db_copy_writes_the_library_into_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("data");
    let out = run(
        &d,
        &["user", "create", "alice", "--admin", "--password-stdin"],
        "alice password\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let copied = dir.path().join("copied");
    std::fs::create_dir(&copied).unwrap();
    let target = copied.join("neo.sqlite3");

    let out = run(&d, &["db", "copy", "--to", target.to_str().unwrap()], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("copied"));

    // The copy is a library of its own, with alice in it.
    assert!(user(&copied, "alice").await.is_some());
    // A second copy to the same file is refused and leaves it alone.
    let before = std::fs::read(&target).unwrap();
    let out = run(&d, &["db", "copy", "--to", target.to_str().unwrap()], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    assert_eq!(std::fs::read(&target).unwrap(), before);
}
