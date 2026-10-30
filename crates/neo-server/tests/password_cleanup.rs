//! Password reclamation must preserve files outside the server's ownership.

use neo_server::roots::{StoredSettings, cleanup_passwords, lock_secrets, write_password};

fn settings(version: Option<String>) -> StoredSettings {
    StoredSettings::WebDav {
        url: "http://synthetic.test/".into(),
        base_path: "/".into(),
        username: "synthetic".into(),
        password_version: version,
        list_concurrency: None,
        auto_scan_interval: None,
    }
}

#[tokio::test]
async fn cleanup_only_removes_legacy_files_superseded_by_a_committed_version() {
    let dir = tempfile::tempdir().unwrap();
    let db = neo_db::testing::open().await;
    let _lock = lock_secrets(dir.path()).unwrap();
    let version = write_password(dir.path(), "replaced", "new synthetic password").unwrap();
    let secrets = dir.path().join("secrets/roots");
    for id in ["unknown", "local", "legacy", "replaced"] {
        std::fs::write(
            secrets.join(format!("{id}.password")),
            "old synthetic password",
        )
        .unwrap();
    }
    for (id, settings) in [
        (
            "local",
            StoredSettings::Local {
                path: dir.path().to_owned(),
                watch: None,
                auto_scan_interval: None,
            },
        ),
        ("legacy", settings(None)),
        ("replaced", settings(Some(version.clone()))),
    ] {
        assert!(
            neo_db::roots::insert_stored(&db.db, &settings.to_stored(id, id), 0)
                .await
                .unwrap()
        );
    }
    let orphan = write_password(dir.path(), "orphan", "unused synthetic password").unwrap();
    cleanup_passwords(dir.path(), &db.db, &[]).await.unwrap();
    for id in ["unknown", "local", "legacy"] {
        assert!(
            secrets.join(format!("{id}.password")).exists(),
            "{id} was not superseded"
        );
    }
    assert!(!secrets.join("replaced.password").exists());
    assert!(
        settings(Some(version))
            .password_path(dir.path(), "replaced")
            .unwrap()
            .exists()
    );
    assert!(
        !settings(Some(orphan))
            .password_path(dir.path(), "orphan")
            .unwrap()
            .exists()
    );
    db.db.close().await;
}

#[tokio::test]
async fn configured_paths_protect_even_superseded_legacy_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = neo_db::testing::open().await;
    let _lock = lock_secrets(dir.path()).unwrap();
    let version = write_password(dir.path(), "remote", "new synthetic password").unwrap();
    let stored = settings(Some(version)).to_stored("remote", "Remote");
    neo_db::roots::insert_stored(&db.db, &stored, 0)
        .await
        .unwrap();
    let legacy = dir.path().join("secrets/roots/remote.password");
    std::fs::write(&legacy, "configured synthetic password").unwrap();
    std::fs::create_dir(dir.path().join("alias")).unwrap();
    let configured = dir.path().join("alias/../secrets/roots/remote.password");
    #[cfg(unix)]
    let configured = {
        let link = dir.path().join("password-link");
        std::os::unix::fs::symlink(configured, &link).unwrap();
        link
    };
    cleanup_passwords(dir.path(), &db.db, &[configured])
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(legacy).unwrap(),
        "configured synthetic password"
    );
    db.db.close().await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server"]
async fn postgres_cleanup_waits_for_in_flight_root_commits() {
    use std::time::Duration;

    use sqlx::{Connection, PgConnection};
    use tokio::time::timeout;

    assert!(neo_db::testing::on_postgres());
    for operation in ["insert", "update", "delete"] {
        let dir = tempfile::tempdir().unwrap();
        let database = neo_db::testing::open().await;
        let neo_db::Location::Postgres(url) = &database.location else {
            unreachable!()
        };
        let replacement = neo_db::Db::open(&database.location).await.unwrap();
        let mut gate = PgConnection::connect(url).await.unwrap();
        let version = write_password(dir.path(), "remote", "new synthetic password").unwrap();
        let new_settings = settings(Some(version));
        let new_path = new_settings.password_path(dir.path(), "remote").unwrap();
        let new_row = new_settings.to_stored("remote", "Remote");
        if operation != "insert" {
            let old_version =
                write_password(dir.path(), "remote", "old synthetic password").unwrap();
            let old = settings(Some(old_version)).to_stored("remote", "Remote");
            neo_db::roots::insert_stored(&database.db, &old, 0)
                .await
                .unwrap();
        }
        // A deferred trigger stops the real edit inside COMMIT, after its
        // row changes but before they become visible to the next instance.
        neo_db::testing::exec(
            &database.db,
            "
            CREATE FUNCTION pause_root_commit() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                PERFORM pg_advisory_xact_lock(987654321);
                RETURN NULL;
            END $$;
            CREATE CONSTRAINT TRIGGER pause_root_commit
            AFTER INSERT OR UPDATE OR DELETE ON root_settings
            DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
            EXECUTE FUNCTION pause_root_commit();
        ",
        )
        .await;
        sqlx::query("SELECT pg_advisory_lock(987654321)")
            .execute(&mut gate)
            .await
            .unwrap();
        let writer = database.db.clone();
        let edit = tokio::spawn(async move {
            match operation {
                "insert" => assert!(
                    neo_db::roots::insert_stored(&writer, &new_row, 1)
                        .await
                        .unwrap()
                ),
                "update" => assert!(
                    neo_db::roots::update_stored(&writer, &new_row, true, 1)
                        .await
                        .unwrap()
                ),
                "delete" => {
                    let mut tx = writer.begin().await.unwrap();
                    assert!(
                        neo_db::roots::remove_stored(&mut tx, "remote", 1)
                            .await
                            .unwrap()
                            .is_some()
                    );
                    tx.commit().await.unwrap();
                }
                _ => unreachable!(),
            }
        });
        timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory'
                     AND classid = 0 AND objid = 987654321 AND NOT granted)",
                )
                .fetch_one(&mut gate)
                .await
                .unwrap();
                if waiting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("edit never reached the commit gate");

        // The old application's filesystem lock has already disappeared.
        let _secrets = lock_secrets(dir.path()).unwrap();
        let cleanup = cleanup_passwords(dir.path(), &replacement, &[]);
        tokio::pin!(cleanup);
        let early = timeout(Duration::from_millis(200), &mut cleanup).await;
        let waited = early.is_err();
        sqlx::query("SELECT pg_advisory_unlock(987654321)")
            .execute(&mut gate)
            .await
            .unwrap();
        timeout(Duration::from_secs(5), edit)
            .await
            .unwrap()
            .unwrap();
        match early {
            Ok(result) => result.unwrap(),
            Err(_) => timeout(Duration::from_secs(5), &mut cleanup)
                .await
                .unwrap()
                .unwrap(),
        }
        assert!(
            waited,
            "cleanup did not wait for the in-flight {operation} commit"
        );
        if operation != "delete" {
            assert_eq!(
                std::fs::read_to_string(new_path).unwrap(),
                "new synthetic password"
            );
            let rows = neo_db::roots::list_stored(&replacement).await.unwrap();
            let committed: StoredSettings = serde_json::from_str(&rows[0].settings).unwrap();
            assert_eq!(committed, new_settings);
        } else {
            assert!(
                neo_db::roots::list_stored(&replacement)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                std::fs::read_dir(dir.path().join("secrets/roots"))
                    .unwrap()
                    .count(),
                0
            );
        }
        gate.close().await.unwrap();
        replacement.close().await;
        database.db.close().await;
    }
}
