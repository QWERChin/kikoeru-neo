#![allow(dead_code)]

use std::{
    fs::File,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    time::{sleep, timeout},
};

const DEADLINE: Duration = Duration::from_secs(20);

fn command(data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kikoeru-neo"));
    command
        .arg("--data-dir")
        .arg(data_dir)
        .env_remove("KIKOERU_DATA_DIR")
        .env_remove("KIKOERU_LISTEN")
        .env_remove("RUST_LOG")
        .env("TOKIO_WORKER_THREADS", "2")
        .kill_on_drop(true);
    command
}

pub async fn user(data_dir: &Path, args: &[&str], password: &str) -> Output {
    let mut child = command(data_dir)
        .arg("user")
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
        .write_all(password.as_bytes())
        .await
        .unwrap();
    timeout(DEADLINE, child.wait_with_output())
        .await
        .expect("user command timed out")
        .unwrap()
}

pub async fn create_admin(data_dir: &Path, name: &str) {
    let output = user(
        data_dir,
        &["create", "--admin", name, "--password-stdin"],
        "initial password\n",
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub struct Server {
    child: Child,
    address: SocketAddr,
    log: PathBuf,
}

impl Server {
    pub async fn start(data_dir: &Path) -> Self {
        let log = data_dir.join("server.log");
        let child = command(data_dir)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let mut server = Self {
            child,
            address: "127.0.0.1:0".parse().unwrap(),
            log,
        };
        // Let the server choose its port; reserving and releasing a test port
        // before spawning it would race with other tests.
        timeout(DEADLINE, async {
            loop {
                let log = std::fs::read_to_string(&server.log).unwrap();
                assert!(
                    server.child.try_wait().unwrap().is_none(),
                    "server exited: {log}"
                );
                if let Some(address) = log
                    .split_inclusive('\n')
                    .filter(|line| line.ends_with('\n'))
                    .find_map(|line| {
                        line.split_once("address=")?
                            .1
                            .split_whitespace()
                            .next()?
                            .parse()
                            .ok()
                    })
                {
                    server.address = address;
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "server did not start: {}",
                std::fs::read_to_string(&server.log).unwrap()
            )
        });
        assert_eq!(
            server.request("GET", "/api/ready", None, None).await,
            (StatusCode::OK, json!({"status": "ready"}))
        );
        server
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut command = Command::new("curl");
        command
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--max-time",
                "10",
                "--noproxy",
                "*",
            ])
            .args(["--write-out", "\n%{http_code}", "--request", method])
            .arg(format!("http://{}{path}", self.address))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(token) = token {
            command.args(["--header", &format!("Authorization: Bearer {token}")]);
        }
        if body.is_some() {
            command.args([
                "--header",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
            ]);
        }
        let mut child = command.spawn().unwrap();
        if let Some(body) = body {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&body).unwrap())
                .await
                .unwrap();
        }
        let output = timeout(DEADLINE, child.wait_with_output())
            .await
            .expect("HTTP request timed out")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        let (body, status) = output.rsplit_once('\n').unwrap();
        let status = StatusCode::from_u16(status.parse().unwrap()).unwrap();
        let body = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(body).unwrap()
        };
        (status, body)
    }

    pub async fn login(&self, name: &str, password: &str) -> String {
        let (status, body) = self
            .request(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"name": name, "password": password})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_owned()
    }

    pub fn pid(&self) -> u32 {
        self.child.id().expect("the server is running")
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// A GET whose body is not JSON, such as a stream or a cover.
    pub async fn get_bytes(&self, path: &str, token: &str) -> (StatusCode, Vec<u8>) {
        let body = tempfile::NamedTempFile::new().unwrap();
        let output = timeout(
            DEADLINE,
            Command::new("curl")
                .args(["--disable", "--silent", "--show-error", "--max-time", "10"])
                .args(["--noproxy", "*", "--write-out", "%{http_code}", "--output"])
                .arg(body.path())
                .args(["--header", &format!("Authorization: Bearer {token}")])
                .arg(self.url(path))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("HTTP request timed out")
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status = String::from_utf8(output.stdout).unwrap();
        (
            StatusCode::from_u16(status.trim().parse().unwrap()).unwrap(),
            std::fs::read(body.path()).unwrap(),
        )
    }

    pub async fn crash(&mut self) {
        self.child.kill().await.unwrap();
    }
}

pub fn data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "listen = \"127.0.0.1:0\"\n").unwrap();
    dir
}
