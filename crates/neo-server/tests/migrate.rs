//! `kikoeru-neo migrate` driven by scripted answers, and as a process.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use neo_server::{cli::MigrateArgs, migrate_cmd};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio_util::sync::CancellationToken;

const OLD: &str = include_str!("fixtures/legacy_0614.sql");

/// Answers in order. A question without a scripted answer fails the test.
#[derive(Default)]
struct Script {
    answers: VecDeque<String>,
    passwords: VecDeque<Option<&'static str>>,
    said: Vec<String>,
    password_questions: Vec<String>,
}

impl migrate_cmd::Prompt for Script {
    fn say(&mut self, text: &str) -> anyhow::Result<()> {
        self.said.push(text.to_owned());
        Ok(())
    }

    fn ask(&mut self, question: &str, default: Option<&str>) -> anyhow::Result<String> {
        let answer = self.answers.pop_front().expect(question);
        Ok(match (answer.as_str(), default) {
            ("", Some(d)) => d.to_owned(),
            _ => answer,
        })
    }

    fn confirm(&mut self, question: &str, _: bool) -> anyhow::Result<bool> {
        Ok(self.answers.pop_front().expect(question) == "y")
    }

    fn password(&mut self, question: &str, _: bool) -> anyhow::Result<Option<String>> {
        self.password_questions.push(question.to_owned());
        Ok(self
            .passwords
            .pop_front()
            .expect(question)
            .map(str::to_owned))
    }
}

async fn old_install(dir: &Path) -> PathBuf {
    let old = dir.join("old");
    fs::create_dir_all(old.join("sqlite")).unwrap();
    fs::create_dir_all(old.join("covers")).unwrap();
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
    old
}

fn args(source: Option<&Path>) -> MigrateArgs {
    MigrateArgs {
        source: source.map(Path::to_owned),
        map_paths: Vec::new(),
        roots: Vec::new(),
        yes: false,
    }
}

#[tokio::test]
async fn asks_its_way_through_a_full_import() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    let target = dir.path().join("new");
    let voice = dir.path().join("voice");
    // The library's works, so the spot check finds them.
    for work in ["RJ123456", "RJ234567"] {
        fs::create_dir_all(voice.join(work)).unwrap();
    }
    let mut script = Script {
        answers: [old.to_str().unwrap(), "voice", voice.to_str().unwrap(), "y"]
            .map(str::to_owned)
            .into(),
        passwords: VecDeque::from([Some("a long enough password"), None]),
        ..Script::default()
    };
    let mut out = Vec::new();
    migrate_cmd::run(
        &args(None),
        &target,
        &CancellationToken::new(),
        &mut script,
        &mut out,
    )
    .await
    .unwrap();

    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("imported into"), "{out}");
    let config = fs::read_to_string(target.join("config.toml")).unwrap();
    assert!(config.contains("id = \"voice\""), "{config}");
    assert!(
        config.contains(&format!("path = \"{}\"", voice.display())),
        "{config}"
    );
    assert!(
        script.said.iter().any(|s| s.contains("guest")),
        "the skipped user is named: {:?}",
        script.said
    );
    let db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(target.join("neo.sqlite3"))
                .read_only(true),
        )
        .await
        .unwrap();
    let with_password: Vec<String> =
        sqlx::query_scalar("SELECT name FROM users WHERE password_hash IS NOT NULL")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(with_password, ["admin"]);
    db.close().await;
}

#[tokio::test]
async fn needs_a_terminal_or_yes() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    let target = dir.path().join("new");
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
            .env_remove("KIKOERU_DATA_DIR")
            .arg("--data-dir")
            .arg(&target)
            .arg("migrate")
            .arg("--source")
            .arg(&old)
            .args(extra)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    let out = run(&[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--yes"));

    // An invalid preset answer is an error, not a question asked forever.
    let out = run(&["--yes", "--root", "Old=bad.id"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ASCII letters"));

    // --yes takes the suggested ID and sets no passwords.
    let out = run(&["--yes", "--root", "Old=voice"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config = fs::read_to_string(target.join("config.toml")).unwrap();
    assert!(config.contains("id = \"voice\""), "{config}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("user set-password"));
}

#[tokio::test]
async fn merges_into_a_used_library_and_stores_the_openlist_password() {
    use std::{io::Write, os::unix::fs::PermissionsExt};

    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    fs::create_dir(old.join("config")).unwrap();
    fs::write(
        old.join("config/config.json"),
        r#"{"rootFolders": [{"name": "Old", "type": "openlist", "url": "http://ol:5244",
            "basePath": "/voice", "username": "u", "password": "OLD-PASSWORD"}]}"#,
    )
    .unwrap();
    let library = dir.path().join("library");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .arg("--data-dir")
        .arg(&library)
        .args(["user", "create", "--admin", "admin", "--password-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a long enough password\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    fs::write(library.join("config.toml"), "# the user's own config\n").unwrap();

    let mut script = Script {
        answers: ["cloud", "y"].map(str::to_owned).into(),
        passwords: VecDeque::from([Some("new openlist password"), None]),
        ..Script::default()
    };
    let mut out = Vec::new();
    migrate_cmd::run(
        &args(Some(&old)),
        &library,
        &CancellationToken::new(),
        &mut script,
        &mut out,
    )
    .await
    .unwrap();

    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("merged user data for 2 works"), "{out}");
    assert!(!out.contains("OLD-PASSWORD"), "{out}");
    // The user's config stays, and the libraries go next to it.
    assert_eq!(
        fs::read_to_string(library.join("config.toml")).unwrap(),
        "# the user's own config\n"
    );
    let roots = fs::read_to_string(library.join("config.from-kikoeru.toml")).unwrap();
    // Live, not commented out, because its password file exists.
    assert!(roots.lines().any(|l| l == "id = \"cloud\""), "{roots}");
    let secret = library.join("secrets/openlist-cloud.password");
    assert_eq!(
        fs::read_to_string(&secret).unwrap(),
        "new openlist password"
    );
    assert_eq!(
        fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn writes_libraries_the_old_config_names_without_works() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    fs::create_dir(old.join("config")).unwrap();
    fs::write(
        old.join("config/config.json"),
        format!(
            r#"{{"rootFolders": [{{"name": "Empty", "path": "{}"}}]}}"#,
            empty.display()
        ),
    )
    .unwrap();
    let target = dir.path().join("new");
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .arg("--data-dir")
        .arg(&target)
        .arg("migrate")
        .arg("--source")
        .arg(&old)
        .args(["--yes", "--root", "Old=voice", "--root", "Empty=empty"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config = fs::read_to_string(target.join("config.toml")).unwrap();
    assert!(config.lines().any(|l| l == "id = \"empty\""), "{config}");
}

#[tokio::test]
async fn shares_one_password_file_per_id_and_writes_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    fs::create_dir(old.join("config")).unwrap();
    fs::write(
        old.join("config/config.json"),
        r#"{"rootFolders": [
            {"name": "Old", "type": "openlist", "url": "http://ol:5244", "basePath": "/v", "username": "u"},
            {"name": "Other", "type": "openlist", "url": "http://ol:5244", "basePath": "/v", "username": "u"}]}"#,
    )
    .unwrap();
    // A relative --data-dir, resolved against where migrate runs.
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .current_dir(dir.path())
        .env_remove("KIKOERU_DATA_DIR")
        .args(["--data-dir", "new", "migrate", "--source"])
        .arg(&old)
        .args(["--yes", "--root", "Old=cloud", "--root", "Other=cloud"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config = fs::read_to_string(dir.path().join("new/config.toml")).unwrap();
    let expected = dir.path().join("new/secrets/openlist-cloud.password");
    assert!(
        config.contains(&format!("password_file = \"{}\"", expected.display())),
        "{config}"
    );
    assert_eq!(config.matches("id = \"cloud\"").count(), 1, "{config}");

    // Asked interactively, the shared password is asked once.
    let target = dir.path().join("second");
    let mut script = Script {
        answers: ["cloud", "cloud", "y"].map(str::to_owned).into(),
        passwords: VecDeque::from([Some("pw"), None, None]),
        ..Script::default()
    };
    migrate_cmd::run(
        &args(Some(&old)),
        &target,
        &CancellationToken::new(),
        &mut script,
        &mut Vec::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(target.join("secrets/openlist-cloud.password")).unwrap(),
        "pw"
    );
}

#[tokio::test]
async fn the_password_goes_to_the_server_whose_entry_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    fs::create_dir(old.join("config")).unwrap();
    // Listed against name order, and on two different servers.
    fs::write(
        old.join("config/config.json"),
        r#"{"rootFolders": [
            {"name": "Zeta", "type": "openlist", "url": "http://server-b", "basePath": "/v", "username": "u"},
            {"name": "Old", "type": "openlist", "url": "http://server-a", "basePath": "/v", "username": "u"}]}"#,
    )
    .unwrap();
    let target = dir.path().join("new");
    let mut script = Script {
        answers: ["cloud", "cloud", "y"].map(str::to_owned).into(),
        passwords: VecDeque::from([Some("password for a"), None, None]),
        ..Script::default()
    };
    migrate_cmd::run(
        &args(Some(&old)),
        &target,
        &CancellationToken::new(),
        &mut script,
        &mut Vec::new(),
    )
    .await
    .unwrap();
    assert!(
        script.password_questions[0].contains("server-a"),
        "{:?}",
        script.password_questions
    );
    let config = fs::read_to_string(target.join("config.toml")).unwrap();
    let live: Vec<&str> = config.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(live.contains(&"url = \"http://server-a\""), "{config}");
    assert!(!live.contains(&"url = \"http://server-b\""), "{config}");
}

#[tokio::test]
async fn asks_again_when_the_old_folder_holds_few_of_its_works() {
    let dir = tempfile::tempdir().unwrap();
    let old = old_install(dir.path()).await;
    // The old config points at a folder that exists but holds none of the
    // library's works, like a disk that is not mounted.
    let empty = dir.path().join("empty");
    let real = dir.path().join("real");
    fs::create_dir(&empty).unwrap();
    for work in ["RJ123456", "RJ234567"] {
        fs::create_dir_all(real.join(work)).unwrap();
    }
    fs::create_dir(old.join("config")).unwrap();
    fs::write(
        old.join("config/config.json"),
        format!(
            r#"{{"rootFolders": [{{"name": "Old", "path": "{}"}}]}}"#,
            empty.display()
        ),
    )
    .unwrap();
    let target = dir.path().join("new");
    let mut script = Script {
        // Refuses the configured folder, then the empty one typed in, then
        // takes the real one.
        answers: [
            "voice",
            "n",
            empty.to_str().unwrap(),
            "n",
            real.to_str().unwrap(),
            "y",
        ]
        .map(str::to_owned)
        .into(),
        passwords: VecDeque::from([None, None]),
        ..Script::default()
    };
    migrate_cmd::run(
        &args(Some(&old)),
        &target,
        &CancellationToken::new(),
        &mut script,
        &mut Vec::new(),
    )
    .await
    .unwrap();
    let config = fs::read_to_string(target.join("config.toml")).unwrap();
    assert!(
        config.contains(&format!("path = \"{}\"", real.display())),
        "{config}"
    );
}
