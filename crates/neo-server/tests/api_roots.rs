//! Adding, editing and removing roots through `/api/admin/roots`. The test
//! app's `lib` and `cloud` roots come from config.toml.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

async fn setup() -> (App, String) {
    let app = app().await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library'), ('cloud', 'Cloud')")
        .await;
    let token = app.login("admin").await;
    (app, token)
}

fn local(id: &str, path: &std::path::Path) -> Value {
    json!({
        "id": id,
        "display_name": "Extra",
        "settings": {"type": "local", "path": path.display().to_string()},
    })
}

/// An OpenList root on a port nothing listens on.
fn unreachable_openlist(id: &str) -> Value {
    json!({
        "id": id,
        "display_name": "Remote",
        "settings": {
            "type": "openlist",
            "url": "http://127.0.0.1:9",
            "base_path": "/voice",
            "username": "u",
            "password": "secret",
        },
    })
}

async fn digest(app: &App) -> Option<String> {
    sqlx::query_scalar("SELECT listing_hash FROM work_locations WHERE id = 1")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap()
}

async fn list(app: &App, token: &str) -> Vec<Value> {
    let (status, body) = app.send(Req::get("/api/admin/roots").token(token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().unwrap().clone()
}

async fn password_path(app: &App, id: &str) -> std::path::PathBuf {
    let stored = neo_db::roots::list_stored(&app.db)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == id)
        .unwrap();
    let settings: neo_server::roots::StoredSettings =
        serde_json::from_str(&stored.settings).unwrap();
    settings.password_path(app.data_dir(), id).unwrap()
}

#[tokio::test]
async fn a_rejected_commit_keeps_the_old_credentials_and_reclaims_only_orphans() {
    let (app, token) = setup().await;
    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(request.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let old = password_path(&app, "remote").await;
    app.exec("CREATE TRIGGER reject_root BEFORE UPDATE ON root_settings BEGIN SELECT RAISE(ABORT, 'injected root commit failure'); END;").await;
    request["settings"]["url"] = json!("http://new.test/");
    request["settings"]["password"] = json!("replacement-password");
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(request),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
    );
    assert_eq!(password_path(&app, "remote").await, old);
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "secret");
    let roots = list(&app, &token).await;
    assert_eq!(roots[2]["settings"]["url"], "http://127.0.0.1:9/");
    let _secrets = neo_server::roots::lock_secrets(app.data_dir()).unwrap();
    neo_server::roots::cleanup_passwords(app.data_dir(), &app.db, &[])
        .await
        .unwrap();
    assert!(old.is_file());
    assert_eq!(
        std::fs::read_dir(app.data_dir().join("secrets/roots"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn password_reuse_checks_the_committed_address_when_the_registry_is_stale() {
    let (app, token) = setup().await;
    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(request.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Model a successful commit whose acknowledgement never reached the
    // handler. The registry still holds the previous committed edit.
    let mut stored = neo_db::roots::list_stored(&app.db).await.unwrap().remove(0);
    let mut settings: Value = serde_json::from_str(&stored.settings).unwrap();
    settings["url"] = json!("http://new.test/");
    settings["password_version"] = json!(
        neo_server::roots::write_password(app.data_dir(), "remote", "replacement-password")
            .unwrap()
    );
    stored.settings = settings.to_string();
    neo_db::roots::update_stored(&app.db, &stored, true, 1)
        .await
        .unwrap();

    request["settings"]
        .as_object_mut()
        .unwrap()
        .remove("password");
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(request),
        )
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    assert_eq!(
        std::fs::read_to_string(password_path(&app, "remote").await).unwrap(),
        "replacement-password"
    );
}

#[tokio::test]
async fn cleanup_keeps_old_files_until_the_reference_change_is_durable() {
    let (app, token) = setup().await;
    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(request.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let old = password_path(&app, "remote").await;
    app.exec("PRAGMA busy_timeout = 0").await;
    let mut reader = app.db.sqlite().unwrap().read.begin().await.unwrap();
    sqlx::query("SELECT * FROM root_settings")
        .fetch_all(&mut *reader)
        .await
        .unwrap();
    request["settings"]["password"] = json!("replacement-password");
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(request),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let new = password_path(&app, "remote").await;
    assert_ne!(old, new);
    assert!(old.is_file(), "a blocked checkpoint must defer deletion");
    assert_eq!(
        std::fs::read_to_string(&new).unwrap(),
        "replacement-password"
    );
    reader.rollback().await.unwrap();
    let _secrets = neo_server::roots::lock_secrets(app.data_dir()).unwrap();
    neo_server::roots::cleanup_passwords(app.data_dir(), &app.db, &[])
        .await
        .unwrap();
    assert!(!old.exists());
    assert!(new.is_file());
}

#[tokio::test]
async fn a_local_root_is_added_listed_and_scanned_without_a_restart() {
    let (app, token) = setup().await;
    let extra = app.data_dir().join("extra");
    std::fs::create_dir_all(extra.join("RJ123456")).unwrap();
    std::fs::write(extra.join("RJ123456/01.mp3"), b"audio").unwrap();

    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(local("extra", &extra)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], "extra");
    assert_eq!(body["source"], "db");
    assert_eq!(body["status"], "unknown");
    assert_eq!(body["work_count"], 0);
    assert_eq!(body["settings"]["type"], "local");

    let roots = list(&app, &token).await;
    let ids: Vec<_> = roots.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["lib", "cloud", "extra"], "config roots come first");

    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "extra"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn invalid_roots_are_rejected() {
    let (app, token) = setup().await;
    let dir = app.data_dir().join("extra");
    std::fs::create_dir_all(&dir).unwrap();
    let file = app.data_dir().join("file");
    std::fs::write(&file, b"").unwrap();
    let cases = [
        local("Extra", &dir),
        local("-extra", &dir),
        local(&"a".repeat(33), &dir),
        local("extra", std::path::Path::new("relative/dir")),
        local("extra", &app.data_dir().join("absent")),
        local("extra", &file),
        json!({"id": "extra", "display_name": "  ",
               "settings": {"type": "local", "path": dir.display().to_string()}}),
        json!({"id": "extra", "display_name": "x".repeat(101),
               "settings": {"type": "local", "path": dir.display().to_string()}}),
        json!({"id": "remote", "display_name": "Remote", "skip_check": true,
               "settings": {"type": "openlist", "url": "http://127.0.0.1:9",
                            "base_path": "/voice", "username": "u"}}),
        json!({"id": "remote", "display_name": "Remote", "skip_check": true,
               "settings": {"type": "openlist", "url": "http://host", "base_path": "/voice",
                            "username": "u", "password": "p\n"}}),
        json!({"id": "remote", "display_name": "Remote", "skip_check": true,
               "settings": {"type": "openlist", "url": "ftp://host", "base_path": "/voice",
                            "username": "u", "password": "p"}}),
        json!({"id": "remote", "display_name": "Remote", "skip_check": true,
               "settings": {"type": "openlist", "url": "http://host", "base_path": "/voice",
                            "username": "u", "password": "p", "list_concurrency": 0}}),
    ];
    for case in cases {
        let (status, body) = app
            .send(
                Req::post("/api/admin/roots")
                    .token(&token)
                    .json(case.clone()),
            )
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(local("lib", &dir)),
        )
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "root_exists");
    let alice = app.login("alice").await;
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&alice)
                .json(local("extra", &dir)),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(list(&app, &token).await.len(), 2);
}

#[tokio::test]
async fn an_unreachable_openlist_is_saved_only_when_asked() {
    let (app, token) = setup().await;
    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(unreachable_openlist("remote")),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    let password = app.data_dir().join("secrets/roots/remote.password");
    assert!(!password.exists(), "nothing is saved when the check fails");

    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["settings"]["password_set"], true);
    assert!(!body.to_string().contains("secret"), "{body}");
    let password = password_path(&app, "remote").await;
    assert_eq!(std::fs::read_to_string(&password).unwrap(), "secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&password).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let roots = list(&app, &token).await;
    assert!(!serde_json::to_string(&roots).unwrap().contains("secret"));

    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(local("remote", app.data_dir())),
        )
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "root_exists");
}

#[tokio::test]
async fn a_stored_root_is_edited_in_place() {
    let (app, token) = setup().await;
    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, _) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let password = password_path(&app, "remote").await;
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, listing_hash)
           VALUES (1, 1, 'remote', CAST('RJ000001' AS BLOB), 'RJ000001', 'digest');",
    )
    .await;

    // No password keeps the saved one.
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(json!({
                    "display_name": "Renamed",
                    "skip_check": true,
                    "settings": {"type": "openlist", "url": "http://127.0.0.1:9",
                                 "base_path": "/other", "username": "u", "link_rate": 2.0},
                })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Renamed");
    assert_eq!(body["settings"]["base_path"], "/other");
    assert_eq!(body["settings"]["link_rate"], 2.0);
    assert_eq!(body["settings"]["password_set"], true);
    assert_eq!(std::fs::read_to_string(&password).unwrap(), "secret");
    assert_eq!(
        digest(&app).await,
        None,
        "a new base_path is walked in full"
    );

    // Only the name and limits change, so the digest stays.
    app.exec("UPDATE work_locations SET listing_hash = 'digest'")
        .await;
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(json!({
                    "display_name": "Renamed again",
                    "skip_check": true,
                    "settings": {"type": "openlist", "url": "http://127.0.0.1:9/",
                                 "base_path": "/other", "username": "u", "link_rate": 3.0},
                })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(digest(&app).await.as_deref(), Some("digest"));

    // A new server address needs the password again, so the saved one is
    // never sent anywhere else.
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(json!({
                    "display_name": "Renamed",
                    "skip_check": true,
                    "settings": {"type": "openlist", "url": "http://198.51.100.7:5244",
                                 "base_path": "/other", "username": "u"},
                })),
        )
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");

    // Turning it into a local root drops the password.
    let dir = app.data_dir().join("extra");
    std::fs::create_dir_all(&dir).unwrap();
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(json!({
                    "display_name": "Local now",
                    "settings": {"type": "local", "path": dir.display().to_string()},
                })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "local");
    assert!(!password.exists());

    // Back to OpenList needs a password again.
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/remote")
                .token(&token)
                .json(json!({
                    "display_name": "Remote",
                    "skip_check": true,
                    "settings": {"type": "openlist", "url": "http://127.0.0.1:9",
                                 "base_path": "/voice", "username": "u"},
                })),
        )
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");

    let local_lib = json!({
        "display_name": "Lib",
        "settings": {"type": "local", "path": dir.display().to_string()},
    });
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/lib")
                .token(&token)
                .json(local_lib.clone()),
        )
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "root_read_only");
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/absent")
                .token(&token)
                .json(local_lib),
        )
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn removing_a_root_marks_its_works_missing() {
    let (app, token) = setup().await;
    let mut request = unreachable_openlist("remote");
    request["skip_check"] = json!(true);
    let (status, _) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let password = password_path(&app, "remote").await;
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Only remote', 'scan', 0, 0),
                  (2, 'RJ000002', 'Also local', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'remote', CAST('RJ000001' AS BLOB), 'RJ000001'),
                  (2, 2, 'remote', CAST('RJ000002' AS BLOB), 'RJ000002'),
                  (3, 2, 'lib', CAST('RJ000002' AS BLOB), 'RJ000002');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind)
           VALUES (1, 1, 1, CAST('01.mp3' AS BLOB), '01.mp3', 'a', 'audio');
         INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
           VALUES ('scan', 'scan:remote', 'queued', '{}', 0, 0);",
    )
    .await;

    let (status, body) = app
        .send(Req::delete("/api/admin/roots/remote").token(&token))
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "scan_running");
    app.exec("UPDATE tasks SET state = 'cancelled'").await;

    let (status, body) = app
        .send(Req::delete("/api/admin/roots/lib").token(&token))
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "root_read_only");
    let (status, _) = app
        .send(Req::delete("/api/admin/roots/remote").token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!password.exists());
    assert_eq!(list(&app, &token).await.len(), 2);

    let missing: Vec<(i64, bool)> =
        sqlx::query_as("SELECT id, missing_at IS NOT NULL FROM works ORDER BY id")
            .fetch_all(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(missing, [(1, true), (2, false)]);
    let track_missing: bool =
        sqlx::query_scalar("SELECT missing_at IS NOT NULL FROM tracks WHERE id = 1")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert!(track_missing);

    let (status, body) = app
        .send(Req::delete("/api/admin/roots/remote").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn removing_the_root_of_a_kept_copy_hands_it_to_another_root() {
    let (app, token) = setup().await;
    let extra = app.data_dir().join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(local("extra", &extra)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // The copy in `extra` is the kept one and the copy in `lib` points at it.
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'extra', CAST('RJ000001' AS BLOB), 'RJ000001'),
                  (2, 1, 'lib', CAST('RJ000001' AS BLOB), 'RJ000001');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size)
           VALUES (1, 1, 1, CAST('01.mp3' AS BLOB), '01.mp3', 'a', 'audio', 5);
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size,
                             duplicate_of)
           VALUES (2, 1, 2, CAST('01.mp3' AS BLOB), '01.mp3', 'a', 'audio', 5, 1);",
    )
    .await;
    let (status, _) = app
        .send(Req::delete("/api/admin/roots/extra").token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let duplicate_of: Option<i64> =
        sqlx::query_scalar("SELECT duplicate_of FROM tracks WHERE id = 2")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(duplicate_of, None, "the copy in lib is the kept one now");
    let missing: bool = sqlx::query_scalar("SELECT missing_at IS NOT NULL FROM works WHERE id = 1")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert!(!missing);
}

#[tokio::test]
async fn a_root_being_scanned_keeps_its_location() {
    let (app, token) = setup().await;
    let extra = app.data_dir().join("extra");
    let other = app.data_dir().join("other");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let (status, _) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(local("extra", &extra)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    app.exec(
        "INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
           VALUES ('scan', 'scan:extra', 'running', '{}', 0, 0);",
    )
    .await;
    let edit = |name: &str, path: &std::path::Path| json!({"display_name": name, "settings": {"type": "local", "path": path.display().to_string()}});
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/extra")
                .token(&token)
                .json(edit("Extra", &other)),
        )
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "scan_running");
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/extra")
                .token(&token)
                .json(edit("Renamed", &extra)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn scan_settings_are_saved_checked_and_reported() {
    let (app, token) = setup().await;
    let extra = app.data_dir().join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut request = local("extra", &extra);
    request["settings"]["watch"] = json!(true);
    request["settings"]["auto_scan_interval"] = json!(600);
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["settings"]["watch"], true);
    assert_eq!(body["settings"]["auto_scan_interval"], 600);
    let next = body["next_scan_at"].as_i64().expect("a next scan time");
    assert!(next >= before + 600_000, "{next} {before}");
    // Nothing runs the watcher in the test app.
    assert_eq!(body["watching"], false);

    // Left out, watch takes the server's default, which is off.
    let (status, body) = app
        .send(
            Req::put("/api/admin/roots/extra")
                .token(&token)
                .json(json!({
                    "display_name": "Extra",
                    "settings": {"type": "local", "path": extra.display().to_string()},
                })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["settings"]["watch"], false);
    assert_eq!(body["settings"]["auto_scan_interval"], Value::Null);
    assert_eq!(body["next_scan_at"], Value::Null);

    let roots = list(&app, &token).await;
    assert_eq!(roots[0]["next_scan_at"], Value::Null);
    assert_eq!(roots[0]["watching"], false);

    let mut too_often = local("other", &extra);
    too_often["settings"]["auto_scan_interval"] = json!(299);
    let mut remote_too_often = unreachable_openlist("remote");
    remote_too_often["skip_check"] = json!(true);
    remote_too_often["settings"]["auto_scan_interval"] = json!(1799);
    for case in [too_often, remote_too_often] {
        let (status, body) = app
            .send(Req::post("/api/admin/roots").token(&token).json(case))
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }

    // A remote root cannot be watched, so asking for it is refused rather
    // than saved without a watcher.
    let mut openlist = unreachable_openlist("remote");
    openlist["skip_check"] = json!(true);
    let mut webdav = openlist.clone();
    webdav["settings"]["type"] = json!("webdav");
    for mut case in [openlist, webdav] {
        let (status, body) = app
            .send(
                Req::post("/api/admin/roots")
                    .token(&token)
                    .json(case.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        case["settings"]["watch"] = json!(true);
        let (status, body) = app
            .send(
                Req::put("/api/admin/roots/remote")
                    .token(&token)
                    .json(json!({"display_name": "Watched", "settings": case["settings"], "skip_check": true})),
            )
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
        let (status, _) = app
            .send(Req::delete("/api/admin/roots/remote").token(&token))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = app
            .send(
                Req::post("/api/admin/roots")
                    .token(&token)
                    .json(case.clone()),
            )
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
        assert!(
            list(&app, &token).await.iter().all(|r| r["id"] != "remote"),
            "nothing was saved"
        );
    }
}
