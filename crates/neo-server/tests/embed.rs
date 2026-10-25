//! The library entry point the desktop shell links against
//! (book/src/desktop.md, "外壳和后端的接口").

use std::{borrow::Cow, process::Command, sync::Arc};

use neo_server::embed::{self, Options, WebAsset};
use serde_json::Value;

fn get(port: u16, path: &str, token: Option<&str>) -> (u16, Value) {
    let mut curl = Command::new("curl");
    curl.args([
        "--disable",
        "--silent",
        "--noproxy",
        "*",
        "--max-time",
        "10",
    ])
    .args(["--write-out", "\n%{http_code}"])
    .arg(format!("http://127.0.0.1:{port}{path}"));
    if let Some(token) = token {
        curl.args(["--header", &format!("Authorization: Bearer {token}")]);
    }
    let out = String::from_utf8(curl.output().unwrap().stdout).unwrap();
    let (body, status) = out.rsplit_once('\n').unwrap();
    (
        status.parse().unwrap(),
        serde_json::from_str(body).unwrap_or(Value::Null),
    )
}

// Multi-threaded, because the blocking curl calls below would otherwise
// stall the runtime the server runs on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starts_on_loopback_signs_in_without_http_and_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    // A server-style listen address is ignored: the desktop backend only
    // ever listens on a random loopback port.
    std::fs::write(data.join("config.toml"), "listen = \"0.0.0.0:8888\"\n").unwrap();
    // Spawned, as the desktop shell does: the start future must be Send
    // for any lifetime.
    let server = tokio::spawn(embed::start(Options {
        data_dir: data.clone(),
        ffmpeg: None,
        ffprobe: None,
        web: None,
    }))
    .await
    .unwrap()
    .unwrap();
    let port = server.port();
    assert_ne!(port, 8888);
    assert_eq!(get(port, "/api/ready", None).0, 200);

    assert!(
        server
            .ensure_admin("me", "a long random password")
            .await
            .unwrap()
    );
    assert!(
        !server
            .ensure_admin("me", "another password here")
            .await
            .unwrap()
    );
    assert!(
        server
            .ensure_admin("bad\nname", "a long random password")
            .await
            .is_err()
    );
    assert!(server.ensure_admin("short", "x").await.is_err());

    let token = server.session_token("me").await.unwrap();
    let (status, me) = get(port, "/api/auth/me", Some(&token));
    assert_eq!(status, 200, "{me}");
    assert_eq!(me["name"], "me");
    assert_eq!(me["is_admin"], true);
    assert!(server.session_token("nobody").await.is_err());

    server.stop().await.unwrap();
    assert_ne!(get(port, "/api/ready", None).0, 200, "stopped");

    // The shell restarts the backend after editing config.toml; the user
    // and its tokens survive.
    let server = embed::start(Options {
        data_dir: data,
        ffmpeg: None,
        ffprobe: None,
        web: None,
    })
    .await
    .unwrap();
    let (status, _) = get(server.port(), "/api/auth/me", Some(&token));
    assert_eq!(status, 200);
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opens_existing_sqlite_with_a_server_postgres_config() {
    let dir = tempfile::tempdir().unwrap();
    let data = neo_server::datadir::DataDir::prepare(dir.path()).unwrap();
    let db = neo_db::Db::open_sqlite(&data.database()).await.unwrap();
    let hash = neo_server::password::hash("a long random password").unwrap();
    let id = neo_db::users::create(&db, "restored", &hash, true, 1, 1)
        .await
        .unwrap();
    db.close().await;
    let config = "[database]\nurl = \"postgres:///kikoeru?host=/run/postgresql\"\n";
    std::fs::write(dir.path().join("config.toml"), config).unwrap();

    let server = embed::start(Options {
        data_dir: dir.path().into(),
        ffmpeg: None,
        ffprobe: None,
        web: None,
    })
    .await
    .unwrap();
    let token = server.session_token("restored").await.unwrap();
    let (status, me) = get(server.port(), "/api/auth/me", Some(&token));
    assert_eq!(status, 200, "{me}");
    assert_eq!(me["id"], id);
    assert_eq!(me["name"], "restored");
    server.stop().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).unwrap(),
        config
    );
}

#[tokio::test]
async fn postgres_config_does_not_hide_other_invalid_settings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[database]\nurl = \"postgres:///kikoeru?host=/run/postgresql\"\n\
         [media]\nheavy_jobs = 0\n",
    )
    .unwrap();
    let result = embed::start(Options {
        data_dir: dir.path().into(),
        ffmpeg: None,
        ffprobe: None,
        web: None,
    })
    .await;
    let error = match result {
        Ok(server) => {
            server.stop().await.unwrap();
            panic!("invalid media.heavy_jobs was accepted");
        }
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<neo_server::config::ConfigError>(),
        Some(neo_server::config::ConfigError::Invalid { param, .. })
            if param == "media.heavy_jobs"
    ));
}

#[test]
fn desktop_data_dirs_follow_each_platform() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    };
    assert_eq!(
        embed::app_data_dir("macos", env(&[("HOME", "/Users/a")])),
        Some("/Users/a/Library/Application Support/kikoeru-neo".into())
    );
    assert_eq!(
        embed::app_data_dir(
            "windows",
            env(&[("APPDATA", "C:\\Users\\a\\AppData\\Roaming")])
        ),
        Some(std::path::PathBuf::from("C:\\Users\\a\\AppData\\Roaming").join("kikoeru-neo"))
    );
    assert_eq!(
        embed::app_data_dir(
            "linux",
            env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/home/a")])
        ),
        Some("/x/kikoeru-neo".into())
    );
    assert_eq!(
        embed::app_data_dir("linux", env(&[("HOME", "/home/a")])),
        Some("/home/a/.local/share/kikoeru-neo".into())
    );
    // A relative XDG_DATA_HOME is invalid by the spec and ignored.
    assert_eq!(
        embed::app_data_dir(
            "linux",
            env(&[("XDG_DATA_HOME", "rel"), ("HOME", "/home/a")])
        ),
        Some("/home/a/.local/share/kikoeru-neo".into())
    );
    assert_eq!(embed::app_data_dir("linux", env(&[])), None);
}

#[tokio::test]
async fn merges_an_old_library_into_the_running_one() {
    use neo_server::import::{
        legacy::{Legacy, Review, User, Work},
        merge::UserMap,
        plan,
    };

    let dir = tempfile::tempdir().unwrap();
    let server = embed::start(Options {
        data_dir: dir.path().join("data"),
        ffmpeg: None,
        ffprobe: None,
        web: None,
    })
    .await
    .unwrap();
    server
        .ensure_admin("me", "a long random password")
        .await
        .unwrap();
    let legacy = Legacy {
        circles: vec![(1, "c".into())],
        works: vec![Work {
            id: 123_456,
            root_folder: "Old".into(),
            dir: "RJ123456".into(),
            title: "t".into(),
            circle_id: 1,
            nsfw: None,
            release: None,
            dl_count: None,
            price: None,
            rate_count: None,
            rate_average: None,
            rate_count_detail: None,
            rank: None,
            memo: None,
            is_custom_meta: None,
            created_at: Some(1),
            updated_at: Some(1),
        }],
        users: vec![User {
            name: "admin".into(),
            group: "administrator".into(),
        }],
        reviews: vec![Review {
            user_name: "admin".into(),
            work_id: 123_456,
            rating: None,
            review_text: None,
            progress: Some("marked".into()),
            created_at: Some(1),
            updated_at: Some(1),
        }],
        ..Legacy::default()
    };
    let (plan, _, outcome) = plan::build(&legacy, None, 1);
    outcome.unwrap();
    // The desktop merges the old admin into its own local account.
    let users = UserMap::from([("admin".into(), "me".into())]);
    let merged = server.merge_legacy(&plan, &users).await.unwrap();
    assert!(merged.users_created.is_empty());
    assert_eq!(merged.works, 1);
    assert_eq!(merged.waiting, 1, "RJ123456 waits for a scan");
    server.stop().await.unwrap();
}

/// The status, content type and body of a GET, for the frontend's files.
fn fetch(port: u16, path: &str) -> (u16, String, String) {
    let out = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--noproxy",
            "*",
            "--max-time",
            "10",
        ])
        .args(["--write-out", "\n%{http_code} %{content_type}"])
        .arg(format!("http://127.0.0.1:{port}{path}"))
        .output()
        .unwrap();
    let out = String::from_utf8(out.stdout).unwrap();
    let (body, meta) = out.rsplit_once('\n').unwrap();
    let (status, kind) = meta.split_once(' ').unwrap();
    (status.parse().unwrap(), kind.to_owned(), body.to_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serves_the_frontend_the_shell_passes_in() {
    fn file(body: &'static str, mime: &str) -> Option<WebAsset> {
        Some(WebAsset {
            body: Cow::Borrowed(body.as_bytes()),
            mime: mime.to_owned(),
            etag: format!("\"{}\"", body.len()),
        })
    }
    let dir = tempfile::tempdir().unwrap();
    let server = embed::start(Options {
        data_dir: dir.path().join("data"),
        ffmpeg: None,
        ffprobe: None,
        web: Some(Arc::new(|path: &str| match path {
            "index.html" => file("<!doctype html><p>from the shell</p>", "text/html"),
            "_app/immutable/app.abc.js" => file("export{}", "text/javascript"),
            _ => None,
        })),
    })
    .await
    .unwrap();
    let port = server.port();
    // With embed-web on as well, the shell's files still win.
    for page in ["/", "/desktop", "/works/3"] {
        let (status, kind, body) = fetch(port, page);
        assert_eq!(status, 200, "{page}");
        assert!(kind.starts_with("text/html"), "{page}: {kind}");
        assert!(body.contains("from the shell"), "{page}: {body}");
    }
    let (status, kind, body) = fetch(port, "/_app/immutable/app.abc.js");
    assert_eq!(
        (status, kind.as_str(), body.as_str()),
        (200, "text/javascript", "export{}")
    );
    // A chunk the source does not have is a 404, not the page.
    let (status, kind, _) = fetch(port, "/_app/immutable/gone.js");
    assert_eq!((status, kind.as_str()), (404, "application/json"));
    server.stop().await.unwrap();
}
