mod process;

use axum::http::StatusCode;
use serde_json::json;

use process::{Server, create_admin, data_dir, user};

#[tokio::test]
async fn symlinked_data_directory_allows_server_start() {
    let dir = data_dir();
    create_admin(dir.path(), "alice").await;
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("data");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();

    let mut server = Server::start(&link).await;
    let token = server.login("alice", "initial password").await;
    let (status, me) = server
        .request("GET", "/api/auth/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["name"], "alice");
    server.crash().await;
}

#[tokio::test]
async fn api_token_survives_a_server_restart() {
    let dir = data_dir();
    create_admin(dir.path(), "alice").await;
    let mut server = Server::start(dir.path()).await;
    let token = server.login("alice", "initial password").await;
    let before = server
        .request("GET", "/api/auth/me", Some(&token), None)
        .await;
    assert_eq!(before.0, StatusCode::OK);

    server.crash().await;
    let mut server = Server::start(dir.path()).await;
    assert_eq!(
        server
            .request("GET", "/api/auth/me", Some(&token), None)
            .await,
        before
    );
    assert_eq!(
        server
            .request("GET", "/api/works", Some(&token), None)
            .await,
        (
            StatusCode::OK,
            json!({"items": [], "page": 1, "page_size": 40, "total": 0})
        )
    );
    server.crash().await;
}

#[tokio::test]
async fn running_server_observes_password_resets_and_user_deletion() {
    let dir = data_dir();
    create_admin(dir.path(), "alice").await;
    let created = user(
        dir.path(),
        &["create", "bob", "--password-stdin"],
        "bob password\n",
    )
    .await;
    assert!(created.status.success(), "{created:?}");
    let mut server = Server::start(dir.path()).await;
    let alice = server.login("alice", "initial password").await;
    let bob = server.login("bob", "bob password").await;

    let reset = user(
        dir.path(),
        &["set-password", "alice", "--password-stdin"],
        "new password\nignored second line\n",
    )
    .await;
    assert!(reset.status.success(), "{reset:?}");
    let (status, body) = server
        .request("GET", "/api/auth/me", Some(&alice), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    assert_eq!(
        server
            .request("GET", "/api/auth/me", Some(&bob), None)
            .await
            .0,
        StatusCode::OK
    );
    let (status, body) = server
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"name": "alice", "password": "initial password"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_credentials");
    let alice = server.login("alice", "new password").await;

    let deleted = user(dir.path(), &["delete", "bob"], "").await;
    assert!(deleted.status.success(), "{deleted:?}");
    let (status, body) = server
        .request("GET", "/api/auth/me", Some(&bob), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    assert_eq!(
        server
            .request("GET", "/api/auth/me", Some(&alice), None)
            .await
            .0,
        StatusCode::OK
    );
    server.crash().await;
}

#[tokio::test]
async fn replacing_the_signing_key_revokes_existing_tokens() {
    let dir = data_dir();
    create_admin(dir.path(), "alice").await;
    let mut server = Server::start(dir.path()).await;
    let old = server.login("alice", "initial password").await;
    assert_eq!(
        server
            .request("GET", "/api/auth/me", Some(&old), None)
            .await
            .0,
        StatusCode::OK
    );
    server.crash().await;
    std::fs::remove_file(dir.path().join("secrets/jwt.key")).unwrap();

    let mut server = Server::start(dir.path()).await;
    let (status, body) = server
        .request("GET", "/api/auth/me", Some(&old), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    let new = server.login("alice", "initial password").await;
    let (status, me) = server
        .request("GET", "/api/auth/me", Some(&new), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["name"], "alice");
    server.crash().await;
}

#[tokio::test]
async fn first_admin_can_be_created_while_server_is_running() {
    let dir = data_dir();
    let mut server = Server::start(dir.path()).await;
    // Setup responses did not check a password and must not exhaust the
    // address's login allowance before an administrator exists.
    for _ in 0..12 {
        let (status, body) = server
            .request(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"name": "alice", "password": "initial password"})),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "setup_required");
    }
    create_admin(dir.path(), "alice").await;
    let token = server.login("alice", "initial password").await;
    let (status, me) = server
        .request("GET", "/api/auth/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["name"], "alice");
    assert_eq!(me["is_admin"], true);
    server.crash().await;
}
