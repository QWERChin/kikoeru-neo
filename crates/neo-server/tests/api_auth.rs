mod common;

use axum::http::StatusCode;
use common::*;
use neo_db::users;
use neo_server::token::{self, Audience, Tokens};
use serde_json::json;

#[tokio::test]
async fn system_endpoints() {
    let app = app().await;
    let (status, body) = app.send(Req::get("/api/health")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"status": "ok"})));
    let (status, body) = app.send(Req::get("/api/ready")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"status": "ready"})));
    let (status, body) = app.send(Req::get("/api/version")).await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape_with_nullable(
        &body,
        &sample("version.json"),
        "version",
        &["git_commit", "build_time"],
    );
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn not_ready_until_started() {
    let app = app_not_ready().await;
    let (status, body) = app.send(Req::get("/api/ready")).await;
    assert_error(status, &body, StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    let (status, body) = app
        .send(Req::post("/api/auth/login").json(json!({"name": "a", "password": "b"})))
        .await;
    assert_error(status, &body, StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    let (status, _) = app.send(Req::get("/api/health")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.send(Req::get("/api/version")).await;
    assert_eq!(status, StatusCode::OK);
    app.state.set_ready();
    let (status, _) = app.send(Req::get("/api/ready")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_routes_return_json_errors() {
    let app = app().await;
    // With the frontend embedded, "/" is the app page (see tests/web.rs).
    let root = (!cfg!(feature = "embed-web")).then_some("/");
    for uri in ["/api/nope", "/api/works/1/nope", "/api"]
        .into_iter()
        .chain(root)
    {
        let (status, body) = app.send(Req::get(uri)).await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    }
    let (status, body) = app.send(Req::get("/api/auth/login")).await;
    assert_error(
        status,
        &body,
        StatusCode::METHOD_NOT_ALLOWED,
        "invalid_request",
    );
}

#[tokio::test]
async fn login_requires_an_admin() {
    let app = app().await;
    let (status, body) = app
        .send(Req::post("/api/auth/login").json(json!({"name": "x", "password": PASSWORD})))
        .await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "setup_required");
    assert_eq!(body, sample("error-setup-required.json"));
    // A regular user alone does not complete setup.
    app.user("bob", false).await;
    let (status, body) = app
        .send(Req::post("/api/auth/login").json(json!({"name": "bob", "password": PASSWORD})))
        .await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "setup_required");
}

#[tokio::test]
async fn login_success() {
    let app = app().await;
    let id = app.user("alice", true).await;
    let before = token::now_secs() as i64 * 1000;
    let (status, body) = app
        .send(Req::post("/api/auth/login").json(json!({"name": "alice", "password": PASSWORD})))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape(&body, &sample("login-response.json"), "login");
    assert_eq!(
        body["user"],
        json!({"id": id, "name": "alice", "is_admin": true})
    );
    let expires_at = body["expires_at"].as_i64().unwrap();
    let ttl = token::API_TTL.as_millis() as i64;
    assert!((before + ttl..=before + ttl + 2000).contains(&expires_at));

    let (status, me) = app
        .send(Req::get("/api/auth/me").token(body["token"].as_str().unwrap()))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&me, &sample("me.json"), "me");
    assert_eq!(me, body["user"]);
}

#[tokio::test]
async fn login_failures_look_the_same() {
    let app = app().await;
    app.user("alice", true).await;
    app.exec("INSERT INTO users (name, password_hash, created_at) VALUES ('imported', NULL, 0)")
        .await;
    for (name, password) in [
        ("alice", "wrong password"),
        ("nobody", PASSWORD),
        ("imported", PASSWORD),
        ("imported", ""),
    ] {
        let (status, body) = app
            .send(Req::post("/api/auth/login").json(json!({"name": name, "password": password})))
            .await;
        assert_error(
            status,
            &body,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        );
        assert_eq!(body["error"]["message"], "用户名或密码不对");
    }
}

#[tokio::test]
async fn malformed_login_is_invalid_request() {
    let app = app().await;
    app.user("alice", true).await;
    for body in ["{", "{\"name\": \"alice\"}", "[]"] {
        let (status, body) = app.send(Req::post("/api/auth/login").raw_body(body)).await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
}

#[tokio::test]
async fn too_many_failed_logins_per_ip() {
    let app = app().await;
    app.user("alice", true).await;
    let attempt = |password: &str, peer: &str| {
        Req::post("/api/auth/login")
            .json(json!({"name": "alice", "password": password}))
            .peer(peer)
    };
    for _ in 0..10 {
        let (status, body) = app.send(attempt("wrong", "192.0.2.1:1000")).await;
        assert_error(
            status,
            &body,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        );
    }
    // The eleventh attempt is refused even with the right password, and the
    // port does not matter.
    let (status, body) = app.send(attempt(PASSWORD, "192.0.2.1:2000")).await;
    assert_error(
        status,
        &body,
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
    );
    // Another address still logs in to the same account.
    let (status, _) = app.send(attempt(PASSWORD, "192.0.2.2:1000")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn guessing_a_name_cannot_lock_out_its_owner() {
    let app = app().await;
    app.user("alice", true).await;
    // 24 failures for alice, spread so no single address reaches its limit.
    for i in 0..24 {
        let (status, _) = app
            .send(
                Req::post("/api/auth/login")
                    .json(json!({"name": "alice", "password": "guess"}))
                    .peer(&format!("192.0.2.{}:{}", i % 3 + 1, 1000 + i)),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // The name is over its limit, so the owner's login is delayed but
    // still succeeds from a fresh address.
    let started = std::time::Instant::now();
    let (status, _) = app
        .send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": PASSWORD}))
                .peer("198.51.100.1:1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(started.elapsed() >= neo_server::http::limiter::DELAY);
}

#[tokio::test]
async fn bad_tokens_are_unauthorized() {
    let app = app().await;
    let id = app.user("alice", true).await;
    let user = users::find_by_id(&app.db, id).await.unwrap().unwrap();
    let now = token::now_secs();
    let valid = Tokens::new(KEY)
        .issue(id, user.token_version, Audience::Api, now)
        .unwrap()
        .token;
    let (status, body) = app.send(Req::get("/api/auth/me").token(&valid)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], id);

    let expired = Tokens::new(KEY)
        .issue(
            id,
            user.token_version,
            Audience::Api,
            now - token::API_TTL.as_secs() - 5,
        )
        .unwrap()
        .token;
    let foreign = Tokens::new(b"some other key of thirty-two byt")
        .issue(id, user.token_version, Audience::Api, now)
        .unwrap()
        .token;
    // A user or version mismatch must not mask missing expiry or signature checks.
    for (name, token) in [("expired", &expired), ("foreign", &foreign)] {
        let claims = jsonwebtoken::dangerous::insecure_decode::<serde_json::Value>(token)
            .unwrap()
            .claims;
        assert_eq!(claims["sub"], user.id.to_string(), "{name} token user");
        assert_eq!(claims["ver"], user.token_version, "{name} token version");
    }
    let unknown_user = Tokens::new(KEY)
        .issue(id + 1, 0, Audience::Api, token::now_secs())
        .unwrap()
        .token;
    for token in [
        None,
        Some("garbage"),
        Some(&expired),
        Some(&foreign),
        Some(&unknown_user),
    ] {
        let mut req = Req::get("/api/auth/me");
        if let Some(t) = token {
            req = req.token(t);
        }
        let (status, body) = app.send(req).await;
        assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
        assert_eq!(body, sample("error-unauthorized.json"));
    }
}

#[tokio::test]
async fn media_token_is_rejected_on_api_routes() {
    let app = app().await;
    app.user("alice", true).await;
    let api = app.login("alice").await;
    let (status, body) = app
        .send(Req::post("/api/auth/media-token").token(&api))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape(&body, &sample("media-token.json"), "media-token");
    let expires_at = body["expires_at"].as_i64().unwrap();
    let ttl = token::MEDIA_TTL.as_millis() as i64;
    let now = token::now_secs() as i64 * 1000;
    assert!((now + ttl - 2000..=now + ttl).contains(&expires_at));
    let media = body["token"].as_str().unwrap();
    for req in [
        Req::get("/api/auth/me"),
        Req::get("/api/works"),
        Req::post("/api/auth/media-token"),
        Req::put("/api/auth/password")
            .json(json!({"current_password": PASSWORD, "new_password": "another password"})),
    ] {
        let (status, body) = app.send(req.token(media)).await;
        assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let (status, _) = app.send(Req::post("/api/auth/media-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn change_password() {
    let app = app().await;
    app.user("alice", true).await;
    let old = app.login("alice").await;
    let change = |current: &str, new: &str| {
        Req::put("/api/auth/password")
            .token(&old)
            .json(json!({"current_password": current, "new_password": new}))
    };
    let (status, body) = app.send(change(PASSWORD, "short")).await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, body) = app.send(change("not it", "a long new password")).await;
    assert_error(
        status,
        &body,
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    );

    let (status, body) = app.send(change(PASSWORD, "a long new password")).await;
    assert_eq!(
        (status, body),
        (StatusCode::NO_CONTENT, serde_json::Value::Null)
    );
    let (status, body) = app.send(Req::get("/api/auth/me").token(&old)).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");

    let (status, body) = app
        .send(Req::post("/api/auth/login").json(json!({"name": "alice", "password": PASSWORD})))
        .await;
    assert_error(
        status,
        &body,
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    );
    let (status, body) = app
        .send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": "a long new password"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .send(Req::get("/api/auth/me").token(body["token"].as_str().unwrap()))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn set_password_command_revokes_tokens() {
    let app = app().await;
    let id = app.user("alice", true).await;
    let api = app.login("alice").await;
    let (_, media) = app
        .send(Req::post("/api/auth/media-token").token(&api))
        .await;
    let media = media["token"].as_str().unwrap().to_owned();
    let before = users::find_by_id(&app.db, id).await.unwrap().unwrap();
    users::set_password(
        &app.db,
        id,
        &neo_server::password::hash("new password!").unwrap(),
    )
    .await
    .unwrap();
    let (status, body) = app.send(Req::get("/api/auth/me").token(&api)).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
    // Check that the media token's version no longer matches the user,
    // which is what the media routes compare.
    let claims = Tokens::new(KEY).verify(&media, Audience::Media).unwrap();
    let after = users::find_by_id(&app.db, id).await.unwrap().unwrap();
    assert_eq!(claims.token_version, before.token_version);
    assert_ne!(claims.token_version, after.token_version);
}

/// A password change already past token checks when an admin resets the
/// password must not overwrite the reset.
#[tokio::test]
async fn self_service_change_loses_to_a_concurrent_reset() {
    let app = app().await;
    let id = app.user("alice", true).await;
    let seen = users::find_by_id(&app.db, id).await.unwrap().unwrap();
    let reset = neo_server::password::hash("reset by admin").unwrap();
    assert!(users::set_password(&app.db, id, &reset).await.unwrap());
    let late = neo_server::password::hash("late self-service").unwrap();
    assert!(
        !users::set_password_if_version(&app.db, id, seen.token_version, &late)
            .await
            .unwrap()
    );
    let now = users::find_by_id(&app.db, id).await.unwrap().unwrap();
    assert_eq!(now.password_hash.as_deref(), Some(reset.as_str()));
    assert_eq!(now.token_version, seen.token_version + 1);
}

#[tokio::test]
async fn deleted_user_token_is_rejected() {
    let app = app().await;
    app.user("alice", true).await;
    app.user("bob", false).await;
    let token = app.login("bob").await;
    assert_eq!(
        users::delete(&app.db, "bob").await.unwrap(),
        users::DeleteOutcome::Deleted
    );
    let (status, body) = app.send(Req::get("/api/auth/me").token(&token)).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn every_failure_kind_counts_against_the_address() {
    let app = app().await;
    app.user("alice", true).await;
    app.user("bob", false).await;
    app.exec("INSERT INTO users (name, password_hash, created_at) VALUES ('imported', NULL, 0)")
        .await;
    // Rotating names does not help: unknown names and accounts without a
    // password count like wrong passwords.
    for i in 0..10 {
        let name = match i % 3 {
            0 => format!("ghost{i}"),
            1 => "imported".to_owned(),
            _ => "alice".to_owned(),
        };
        let (status, _) = app
            .send(
                Req::post("/api/auth/login")
                    .json(json!({"name": name, "password": "wrong"}))
                    .peer("192.0.2.1:1"),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let login = |peer: &str| {
        Req::post("/api/auth/login")
            .json(json!({"name": "bob", "password": PASSWORD}))
            .peer(peer)
    };
    let (status, body) = app.send(login("192.0.2.1:1")).await;
    assert_error(
        status,
        &body,
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
    );
    let (status, _) = app.send(login("192.0.2.2:1")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forwarded_for_is_trusted_only_from_configured_proxies() {
    let app = app().await;
    app.user("alice", true).await;
    for i in 0..10 {
        let (status, _) = app
            .send(
                Req::post("/api/auth/login")
                    .json(json!({"name": format!("ghost{i}"), "password": "x"}))
                    .peer("192.0.2.1:1")
                    .header("x-forwarded-for", &format!("198.51.100.{i}")),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // 192.0.2.1 is not a configured proxy, so its header was ignored and
    // all ten failures counted against 192.0.2.1 itself.
    let ok = |peer: &str, forwarded: &str| {
        Req::post("/api/auth/login")
            .json(json!({"name": "alice", "password": PASSWORD}))
            .peer(peer)
            .header("x-forwarded-for", forwarded)
    };
    let (status, _) = app.send(ok("192.0.2.1:1", "203.0.113.9")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // Through the trusted proxy 127.0.0.1, the last entry names the client.
    // A client cannot escape by prepending its own entries.
    for i in 0..10 {
        let (status, _) = app
            .send(
                Req::post("/api/auth/login")
                    .json(json!({"name": format!("other{i}"), "password": "x"}))
                    .peer("127.0.0.1:1")
                    .header("x-forwarded-for", &format!("10.0.0.{i}, 203.0.113.7")),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = app.send(ok("127.0.0.1:1", "1.2.3.4, 203.0.113.7")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, _) = app.send(ok("127.0.0.1:1", "203.0.113.8")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_trusted_ipv4_proxy_is_recognised_on_a_dual_stack_listener() {
    let app = app().await;
    app.user("alice", true).await;
    // Listening on [::], an IPv4 proxy connects as ::ffff:127.0.0.1.
    for i in 0..10 {
        let (status, _) = app
            .send(
                Req::post("/api/auth/login")
                    .json(json!({"name": format!("ghost{i}"), "password": "x"}))
                    .peer("[::ffff:127.0.0.1]:1")
                    .header("x-forwarded-for", "203.0.113.7"),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = app
        .send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": PASSWORD}))
                .peer("[::ffff:127.0.0.1]:1")
                .header("x-forwarded-for", "203.0.113.8"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "another client is not blocked");
}

#[tokio::test]
async fn password_change_is_rate_limited() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    let change = |current: &str, peer: &str| {
        Req::put("/api/auth/password")
            .token(&token)
            .peer(peer)
            .json(json!({"current_password": current, "new_password": "a long new password"}))
    };
    for _ in 0..10 {
        let (status, body) = app.send(change("guess", "192.0.2.1:1")).await;
        assert_error(
            status,
            &body,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        );
    }
    let (status, body) = app.send(change(PASSWORD, "192.0.2.1:2")).await;
    assert_error(
        status,
        &body,
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
    );
    // Failed logins and failed password changes share the address count.
    let (status, _) = app
        .send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": PASSWORD}))
                .peer("192.0.2.1:3"),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, _) = app.send(change(PASSWORD, "192.0.2.2:1")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn token_for_deleted_account_never_matches_a_new_one() {
    let app = app().await;
    app.user("alice", true).await;
    let bob = app.user("bob", false).await;
    let token = app.login("bob").await;
    let bob_version = users::find_by_id(&app.db, bob)
        .await
        .unwrap()
        .unwrap()
        .token_version;
    users::delete(&app.db, "bob").await.unwrap();

    // SQLite would hand out bob's ID again without AUTOINCREMENT.
    let carol = app.user("carol", false).await;
    assert!(carol > bob);
    let (status, _) = app.send(Req::get("/api/auth/me").token(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Even a row that does get bob's ID back, as after restoring a backup
    // into a fresh database, starts from a new random token_version.
    let version = neo_server::token::initial_version().unwrap();
    assert_ne!(version, bob_version);
    app.exec(&format!(
        "INSERT INTO users (id, name, password_hash, token_version, created_at)
         VALUES ({bob}, 'mallory', NULL, {version}, 0)"
    ))
    .await;
    let (status, _) = app.send(Req::get("/api/auth/me").token(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_guesses_from_one_source_stop_at_the_limit() {
    let app = std::sync::Arc::new(app().await);
    app.user("alice", true).await;
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..50 {
        let app = app.clone();
        tasks.spawn(async move {
            app.send(
                Req::post("/api/auth/login")
                    .json(json!({"name": "alice", "password": format!("guess {i}")}))
                    .peer(&format!("192.0.2.1:{}", 1000 + i)),
            )
            .await
            .0
        });
    }
    let mut verified = 0;
    let mut refused = 0;
    while let Some(status) = tasks.join_next().await {
        match status.unwrap() {
            StatusCode::UNAUTHORIZED => verified += 1,
            StatusCode::TOO_MANY_REQUESTS => refused += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert_eq!(
        verified, 10,
        "only ten attempts may reach the password check"
    );
    assert_eq!(refused, 40);
}

#[tokio::test]
async fn a_successful_login_does_not_erase_previous_failures() {
    let app = app().await;
    app.user("alice", true).await;
    let attempt = |password: &str| {
        Req::post("/api/auth/login")
            .json(json!({"name": "alice", "password": password}))
            .peer("192.0.2.1:1000")
    };
    for _ in 0..9 {
        let (status, body) = app.send(attempt("wrong")).await;
        assert_error(
            status,
            &body,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        );
    }
    let (status, body) = app.send(attempt(PASSWORD)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app.send(attempt("wrong")).await;
    assert_error(
        status,
        &body,
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    );
    let (status, body) = app.send(attempt(PASSWORD)).await;
    assert_error(
        status,
        &body,
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
    );
}

#[tokio::test]
async fn every_answer_refuses_framing_and_api_answers_are_not_stored() {
    use axum::{
        body::Body,
        http::{Request, header},
    };
    use tower::ServiceExt;
    let app = app().await;
    app.user("alice", true).await;
    let login = Request::post("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .extension(axum::extract::ConnectInfo(
            "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::from(
            json!({"name": "alice", "password": PASSWORD}).to_string(),
        ))
        .unwrap();
    let response = app.router.clone().oneshot(login).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "a token answer");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; frame-ancestors 'none'",
        "an API answer loads nothing"
    );
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    // Errors and page paths get them too, and a route's own caching rule
    // is kept.
    for uri in ["/api/works", "/api/nope", "/library"] {
        let (_, headers, _) = app.send_raw(Req::get(uri)).await;
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{uri}");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{uri}");
    }
    let (_, headers, _) = app.send_raw(Req::get("/library")).await;
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'",
        "the page brings its own policy"
    );
    assert_ne!(
        headers
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap()),
        Some("no-store"),
        "page paths keep the frontend's own rule"
    );
}

#[tokio::test]
async fn logging_out_everywhere_revokes_every_token_of_that_user_only() {
    let app = app().await;
    app.user("alice", true).await;
    app.user("bob", false).await;
    let here = app.login("alice").await;
    let there = app.login("alice").await;
    let bob = app.login("bob").await;
    let (_, media) = app
        .send(Req::post("/api/auth/media-token").token(&there))
        .await;
    let media = media["token"].as_str().unwrap().to_owned();

    // A media token cannot do it.
    let (status, body) = app
        .send(Req::post("/api/auth/logout-all").token(&media))
        .await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");

    let (status, body) = app
        .send(Req::post("/api/auth/logout-all").token(&here))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for token in [&here, &there] {
        let (status, _) = app.send(Req::get("/api/auth/me").token(token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = app
        .send(Req::get(&format!("/api/works/1/cover?token={media}")))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the media token too");

    let (status, _) = app.send(Req::get("/api/auth/me").token(&bob)).await;
    assert_eq!(status, StatusCode::OK, "other users stay signed in");
    let again = app.login("alice").await;
    let (status, _) = app.send(Req::get("/api/auth/me").token(&again)).await;
    assert_eq!(status, StatusCode::OK);
}

std::thread_local! {
    static LOG: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// This test thread's log lines. One global subscriber writes each thread's
/// lines to that thread, because thread-local subscribers in parallel tests
/// race on tracing's callsite cache and lose events now and then.
struct Log;

impl Log {
    fn start() -> Self {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            tracing_subscriber::fmt()
                .with_writer(|| Log)
                .with_ansi(false)
                .without_time()
                .init();
        });
        LOG.with(|log| log.borrow_mut().clear());
        Log
    }

    /// The lines other than the access log.
    fn lines(&self) -> Vec<String> {
        let bytes = LOG.with(|log| log.borrow().clone());
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("finished processing request"))
            .map(str::to_owned)
            .collect()
    }
}

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        LOG.with(|log| log.borrow_mut().extend_from_slice(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn logins_are_logged_with_name_and_address_but_no_password() {
    let app = app().await;
    app.user("alice", true).await;
    let log = Log::start();
    let attempt = |name: &str, password: &str| {
        Req::post("/api/auth/login")
            .json(json!({"name": name, "password": password}))
            .peer("192.0.2.7:1000")
    };
    app.send(attempt("alice", "hunter2-wrong")).await;
    app.send(attempt("alice", PASSWORD)).await;
    app.send(attempt("eve\n INFO forged line", "x")).await;
    let lines = log.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with(" WARN"), "{lines:?}");
    assert!(lines[0].contains("login failed"), "{lines:?}");
    assert!(lines[0].contains("name=\"alice\""), "{lines:?}");
    assert!(lines[0].contains("ip=192.0.2.7"), "{lines:?}");
    assert!(lines[1].starts_with(" INFO"), "{lines:?}");
    assert!(lines[1].contains("logged in"), "{lines:?}");
    assert!(
        lines[2].contains(r#"name="eve\n INFO forged line""#),
        "{lines:?}"
    );
    for line in &lines {
        assert!(
            !line.contains("hunter2") && !line.contains(PASSWORD),
            "{lines:?}"
        );
    }
}

#[tokio::test]
async fn refused_attempts_and_wrong_current_passwords_are_warnings() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    let log = Log::start();
    let (status, _) = app
        .send(
            Req::put("/api/auth/password")
                .token(&token)
                .json(json!({"current_password": "wrong", "new_password": "a new password"}))
                .peer("192.0.2.8:1000"),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for _ in 0..10 {
        app.send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": "wrong"}))
                .peer("192.0.2.9:1000"),
        )
        .await;
    }
    let (status, _) = app
        .send(
            Req::post("/api/auth/login")
                .json(json!({"name": "alice", "password": PASSWORD}))
                .peer("192.0.2.9:1000"),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let lines = log.lines();
    assert!(
        lines[0].contains("wrong current password") && lines[0].contains("ip=192.0.2.8"),
        "{lines:?}"
    );
    let refused = lines.last().unwrap();
    assert!(refused.starts_with(" WARN"), "{lines:?}");
    assert!(
        refused.contains("refused") && refused.contains("ip=192.0.2.9"),
        "{lines:?}"
    );
}
