//! The `--dry-run --json` output and import errors against the samples in
//! `contract/import/`. They are not an HTTP API, but the desktop shell and
//! scripts read them (book/src/contract.md, "导入的 JSON 输出").

mod common;

use std::{fs, path::Path, process::Command};

use common::{assert_same_shape, assert_same_shape_with_nullable, sample_in};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const OLD: &str = include_str!("fixtures/legacy_0614.sql");

fn assert_preflight_shape(body: &Value, sample: &Value) {
    // These nullable report fields have non-null values in the sample.
    assert_same_shape_with_nullable(
        body,
        sample,
        "preflight",
        &[
            "refusal",
            "report.source",
            "report.source.config",
            "report.source.covers",
            "report.source.settings",
            "report.source.settings.version",
            "report.source.settings.dbUseDefaultPath",
            "report.source.settings.databaseFolderDir",
            "report.source.settings.coverFolderDir",
            "report.source.settings.rootFolders[].path",
            "report.last_migration",
            "report.old_roots[].old_path",
            "report.old_roots[].path",
            "report.old_roots[].sample",
        ],
    );
}

/// The install the sample was made from: a database, one cover, a config
/// with one media library, and that library's folder on this machine.
async fn install(dir: &Path) {
    let old = dir.join("old");
    for sub in ["sqlite", "covers", "config"] {
        fs::create_dir_all(old.join(sub)).unwrap();
    }
    fs::create_dir_all(dir.join("voice")).unwrap();
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
    fs::write(old.join("covers/RJ123456_img_main.jpg"), "jpeg").unwrap();
    fs::write(
        old.join("config/config.json"),
        r#"{"version":"0.6.14","dbUseDefaultPath":true,"databaseFolderDir":"/usr/src/kikoeru/sqlite",
            "coverFolderDir":"/usr/src/kikoeru/covers",
            "rootFolders":[{"name":"Old","path":"/usr/src/kikoeru/VoiceWork"}]}"#,
    )
    .unwrap();
}

fn dry_run(source: &Path, extra: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"))
        .env_remove("KIKOERU_DATA_DIR")
        .args(["import-legacy", "--dry-run", "--json", "--source"])
        .arg(source)
        .args(extra)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code(), body)
}

#[test]
fn required_preflight_fields_cannot_be_null() {
    let sample = sample_in("import", "preflight.json");
    for pointer in [
        "/report/preflight",
        "/report/preflight/histories_valid",
        "/report/preflight/backslash_dirs",
        "/report/source/config/origin",
        "/report/issues",
        "/report/issues/0",
        "/report/old_roots/0",
        "/report/old_roots/0/sample/checked",
        "/refusal/0",
    ] {
        let mut body = sample.clone();
        *body.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(
            std::panic::catch_unwind(|| assert_preflight_shape(&body, &sample)).is_err(),
            "accepted null at {pointer}"
        );
    }
}

#[test]
fn a_preflight_without_refusal_has_the_sample_shape() {
    let sample = sample_in("import", "preflight.json");
    let mut body = sample.clone();
    body["refusal"] = Value::Null;
    assert_preflight_shape(&body, &sample);
}

#[test]
fn a_nullable_preflight_field_still_checks_its_non_null_type() {
    let sample = sample_in("import", "preflight.json");
    let mut body = sample.clone();
    body["refusal"] = serde_json::json!(false);
    assert!(std::panic::catch_unwind(|| assert_preflight_shape(&body, &sample)).is_err());
}

#[test]
fn preflight_root_samples_can_be_null_without_skipping_other_roots() {
    let sample = sample_in("import", "preflight.json");
    let mut body = sample.clone();
    let roots = body["report"]["old_roots"].as_array_mut().unwrap();
    roots.push(roots[0].clone());
    roots[1]["sample"] = Value::Null;
    assert_preflight_shape(&body, &sample);

    body["report"]["old_roots"][0]["sample"]["checked"] = Value::Null;
    assert!(std::panic::catch_unwind(|| assert_preflight_shape(&body, &sample)).is_err());
}

#[tokio::test]
async fn the_dry_run_report_has_the_sample_shape() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path()).await;
    let map = format!(
        "/usr/src/kikoeru/VoiceWork={}",
        dir.path().join("voice").display()
    );
    let (code, body) = dry_run(
        &dir.path().join("old"),
        &["--root", "Old=voice", "--map-path", &map],
    );
    assert_eq!(code, Some(0), "{body}");
    assert_preflight_shape(&body, &sample_in("import", "preflight.json"));
    assert_eq!(body["report"]["source"]["config"]["origin"], "default");
    assert_eq!(
        body["report"]["old_roots"][0]["path"]["origin"],
        "config:rootFolders"
    );
}

#[tokio::test]
async fn errors_have_the_sample_shape_and_a_code() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path()).await;
    let sample = sample_in("import", "error.json");

    let (code, body) = dry_run(&dir.path().join("missing"), &[]);
    assert_eq!(code, Some(1));
    assert_same_shape(&body, &sample, "error");
    assert_eq!(body["error"]["code"], "source_not_found", "{body}");

    let db = dir.path().join("old/sqlite/db.sqlite3");
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&db))
        .await
        .unwrap();
    sqlx::raw_sql("ALTER TABLE t_work DROP COLUMN is_custom_meta")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let (_, body) = dry_run(&dir.path().join("old"), &[]);
    assert_eq!(body["error"]["code"], "too_old", "{body}");

    fs::write(
        dir.path().join("old/config/config.json"),
        r#"{"sqliteType": "mysql"}"#,
    )
    .unwrap();
    let (_, body) = dry_run(&dir.path().join("old"), &[]);
    assert_eq!(body["error"]["code"], "mysql", "{body}");
}
