//! Publication between prepare's cache lookup and task admission.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use futures_util::{poll, stream};
use http_body_util::BodyExt;
use neo_db::{tasks, testing};
use neo_media::{CacheKey, HlsMode, TranscodeCache};
use neo_server::{
    http::{AppState, Media, rebuild_task, router},
    tasks::{Lane, TaskSpec},
    token::{Audience, Tokens, now_secs},
};
use neo_storage::{
    Access, Entry, RawPath, RemoteFile, RemoteOpen, RemoteResponse, Source, Storage, StorageError,
};
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;

const SOURCE: &[u8] = b"synthetic source";
const SEGMENT: &[u8] = b"synthetic cached segment";
const MTIME: i64 = 1_700_000_000_000;
const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Gate {
    reached: Notify,
    resume: Notify,
}

struct Remote {
    size: u64,
    reads: Arc<AtomicUsize>,
    gate: Mutex<Option<Arc<Gate>>>,
}

struct Opener(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl RemoteOpen for Opener {
    async fn open(&self, _: &HeaderMap) -> Result<RemoteResponse, StorageError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(RemoteResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Box::pin(stream::once(async {
                Ok(bytes::Bytes::from_static(SOURCE))
            })),
        })
    }
}

#[async_trait::async_trait]
impl Storage for Remote {
    async fn list(&self, _: &RawPath) -> Result<Vec<Entry>, StorageError> {
        unreachable!("prepare does not list directories")
    }

    async fn stat(&self, _: &RawPath) -> Result<Entry, StorageError> {
        unreachable!("a published cache needs no restat")
    }

    async fn access(&self, _: &RawPath) -> Result<Access, StorageError> {
        unreachable!("prepared playback reads the cache")
    }

    async fn source(&self, _: &RawPath) -> Result<Source, StorageError> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.reached.notify_one();
            gate.resume.notified().await;
        }
        Ok(Source::Remote(RemoteFile {
            size: Some(self.size),
            mtime_ms: Some(MTIME),
            opener: Arc::new(Opener(self.reads.clone())),
        }))
    }
}

struct Fixture {
    db: testing::TestDb,
    dir: tempfile::TempDir,
    state: AppState,
    cache: TranscodeCache,
    remote: Arc<Remote>,
    key: CacheKey,
    token: String,
}

impl Fixture {
    async fn new(size: u64, floor: u64, mode: HlsMode) -> Self {
        let db = testing::open().await;
        testing::exec(
            &db.db,
            &format!(
                "INSERT INTO roots (id, display_name) VALUES ('cloud', 'Cloud');
             INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
               VALUES (1, 'RJ000001', 'Synthetic work', 'scan', 0, 0);
             INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
               VALUES (1, 1, 'cloud', CAST('RJ001' AS BLOB), 'RJ001');
             INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key,
                                 kind, size, mtime, probe)
               VALUES (1, 1, 1, CAST('movie.mkv' AS BLOB), 'movie.mkv', 'a', 'video',
                       {size}, {MTIME}, '{{\"format\":\"matroska\",\"duration_ms\":1000,
                       \"audio\":{{\"codec\":\"aac\"}},\"video\":{{\"codec\":\"h264\"}}}}');"
            ),
        )
        .await;
        let user = neo_db::users::create(&db.db, "alice", "unused hash", false, 1, 0)
            .await
            .unwrap();
        let tokens = Tokens::new(b"test signing key, 32 bytes long!");
        let token = tokens
            .issue(user, 1, Audience::Api, now_secs())
            .unwrap()
            .token;
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), u64::MAX, floor).unwrap();
        let remote = Arc::new(Remote {
            size,
            reads: Arc::new(AtomicUsize::new(0)),
            gate: Mutex::new(None),
        });
        let state = AppState::new(db.db.clone(), tokens, Vec::new())
            .with_storages([("cloud".to_owned(), remote.clone() as Arc<dyn Storage>)].into())
            .with_media(Media {
                cache: cache.clone(),
                probe_slots: Arc::new(Semaphore::new(1)),
                queue_limit: 32,
                job_timeout: DEADLINE,
                remote_analysis_delay: Duration::ZERO,
                video_transcode: true,
            });
        state.set_ready();
        Self {
            db,
            dir,
            state,
            cache,
            remote,
            key: CacheKey {
                root_id: "cloud".to_owned(),
                source_version: 0,
                path: b"RJ001/movie.mkv".to_vec(),
                size,
                mtime: Some(MTIME),
                params: mode.params(),
            },
            token,
        }
    }

    fn spec(&self, mode: HlsMode) -> TaskSpec {
        let key = CacheKey {
            params: mode.params(),
            ..self.key.clone()
        };
        TaskSpec {
            kind: "transcode",
            dedupe_key: Some(format!("transcode:{}", key.digest())),
            payload: json!({
                "track_id": 1, "key": key.digest(), "params": key.params,
                "output": if mode == HlsMode::Copy {
                    "hls-copy"
                } else {
                    "hls-transcode"
                },
            }),
            lane: Lane::Heavy,
            timeout: Some(DEADLINE),
            queue_limit: None,
        }
    }

    async fn enqueue(&self, mode: HlsMode) -> i64 {
        let spec = self.spec(mode);
        let tasks::Enqueued::Created(id) = tasks::enqueue(
            &self.db.db,
            spec.kind,
            spec.dedupe_key.as_deref(),
            &spec.payload.to_string(),
            None,
            neo_db::now_ms(),
        )
        .await
        .unwrap() else {
            panic!("expected a new task")
        };
        id
    }

    fn publish(&self, mode: HlsMode) {
        let key = CacheKey {
            params: mode.params(),
            ..self.key.clone()
        };
        let tmp = self.dir.path().join("tmp/finished.hls");
        std::fs::create_dir(&tmp).unwrap();
        std::fs::write(
            tmp.join("index.m3u8"),
            b"#EXTM3U\n#EXTINF:1.0,\nseg-00000.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
        std::fs::write(tmp.join("seg-00000.ts"), SEGMENT).unwrap();
        std::fs::rename(
            tmp,
            self.dir
                .path()
                .join(format!("transcode/{}.hls", key.digest())),
        )
        .unwrap();
    }

    async fn done(&self, id: i64) {
        loop {
            let task = tasks::get(&self.db.db, id).await.unwrap().unwrap();
            if task.state != "queued" && task.state != "running" {
                assert_eq!(task.state, "done", "{task:?}");
                let result: Value = serde_json::from_str(task.result.as_deref().unwrap()).unwrap();
                assert_eq!(result["url"], "/api/tracks/1/hls/index.m3u8");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn assert_playable(&self) {
        let response = router(self.state.clone())
            .oneshot(
                Request::get("/api/tracks/1/hls/index.m3u8")
                    .header("authorization", format!("Bearer {}", self.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let playlist = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(playlist.contains("#EXT-X-ENDLIST"));
        let segment = playlist
            .lines()
            .find(|line| line.contains("/seg-"))
            .unwrap();
        let response = router(self.state.clone())
            .oneshot(
                Request::get(format!("/api/tracks/1/hls/{segment}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            SEGMENT
        );
        assert_eq!(
            self.remote.reads.load(Ordering::SeqCst),
            0,
            "the completed task's source must not be downloaded again"
        );
        self.state.scheduler().shutdown(DEADLINE).await;
    }
}

async fn completion_race(size: u64, requested: HlsMode, published: HlsMode) {
    let f = Fixture::new(size, 0, requested).await;
    let first = f.enqueue(published).await;
    assert!(
        tasks::start(&f.db.db, first, neo_db::now_ms())
            .await
            .unwrap()
    );
    let gate = Arc::new(Gate::default());
    *f.remote.gate.lock().unwrap() = Some(gate.clone());
    let request = Request::post("/api/tracks/1/prepare")
        .header("authorization", format!("Bearer {}", f.token))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "can_play": ["video/mp4"], "force_fallback": requested == HlsMode::Transcode,
            })
            .to_string(),
        ))
        .unwrap();
    let mut pending = Box::pin(router(f.state.clone()).oneshot(request));
    tokio::select! {
        () = gate.reached.notified() => {},
        result = &mut pending => panic!("prepare bypassed the source gate: {result:?}"),
    }
    // With the probe already stored, the first await after source() is
    // active_id. Exhaust its pool and poll past the initial cache miss.
    let mut connections: Vec<Box<dyn Send>> = Vec::new();
    neo_db::with_pool!(&f.db.db, read, pool => {
        for _ in 0..pool.options().get_max_connections() {
            connections.push(Box::new(pool.acquire().await.unwrap()));
        }
    });
    gate.resume.notify_one();
    assert!(poll!(&mut pending).is_pending());
    assert!(f.cache.lookup(&f.key).unwrap().is_none());

    // Model A's final rename and done transition while B cannot query it.
    f.publish(published);
    assert!(
        tasks::finish(
            &f.db.db,
            first,
            tasks::EndState::Done,
            None,
            None,
            neo_db::now_ms()
        )
        .await
        .unwrap()
    );
    drop(connections);
    let response = pending.await.unwrap();
    let status = response.status();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    if size == SOURCE.len() as u64 {
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let second = body["task_id"].as_i64().unwrap();
        assert_ne!(second, first);
        f.done(second).await;
    } else {
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["state"], "ready");
    }
    f.assert_playable().await;
}

#[tokio::test]
async fn completed_prepare_does_not_download_the_remote_source_again() {
    tokio::time::timeout(DEADLINE, async {
        for (requested, published) in [
            (HlsMode::Copy, HlsMode::Copy),
            (HlsMode::Transcode, HlsMode::Transcode),
            (HlsMode::Copy, HlsMode::Transcode),
        ] {
            completion_race(SOURCE.len() as u64, requested, published).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn completed_prepare_does_not_require_space_for_another_copy() {
    tokio::time::timeout(DEADLINE, async {
        for published in [HlsMode::Copy, HlsMode::Transcode] {
            completion_race(i64::MAX as u64, HlsMode::Copy, published).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_transcode_uses_published_cache_even_on_a_full_disk() {
    tokio::time::timeout(DEADLINE, async {
        for (size, floor) in [(SOURCE.len() as u64, u64::MAX), (i64::MAX as u64, 0)] {
            let f = Fixture::new(size, floor, HlsMode::Copy).await;
            let id = f.enqueue(HlsMode::Copy).await;
            f.publish(HlsMode::Copy);
            // Recovery uses the same job as a fresh submission after A ends.
            f.state
                .scheduler()
                .recover(|row| rebuild_task(&f.state, row))
                .await
                .unwrap();
            f.done(id).await;
            f.assert_playable().await;
        }
    })
    .await
    .unwrap();
}
