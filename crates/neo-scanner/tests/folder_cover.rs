//! Covers taken from the works' own folders, through local and OpenList backends.
//! Every name and image is synthetic.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use axum::{
    Json, Router,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use neo_db::{
    Db,
    testing::{self, TestDb, exec},
};
use neo_scanner::{
    NoObserver, Observer, Root, Stage,
    metadata::{CoverDirs, FolderCoverSummary, fill_from_folders, store_folder_cover},
    scan_root,
};
use neo_storage::{LocalStorage, OpenListStorage};
use serde_json::{Value, json};

const NOW: i64 = 1_800_000_000_000;

struct Lib {
    _dir: tempfile::TempDir,
    _db: TestDb,
    base: PathBuf,
    db: Db,
    dirs: CoverDirs,
}

impl Lib {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_owned();
        let t = testing::open().await;
        let db = t.db.clone();
        fs::create_dir_all(base.join("lib")).unwrap();
        let dirs = CoverDirs::new(base.join("covers"), base.join("tmp"));
        fs::create_dir_all(&dirs.covers).unwrap();
        fs::create_dir_all(&dirs.tmp).unwrap();
        fs::create_dir_all(base.join("other")).unwrap();
        exec(
            &db,
            "INSERT INTO roots (id, display_name) VALUES ('lib', 'lib'), ('other', 'other')",
        )
        .await;
        Self {
            _dir: dir,
            _db: t,
            base,
            db,
            dirs,
        }
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        self.write_in("lib", rel, bytes);
    }

    fn write_in(&self, root: &str, rel: &str, bytes: &[u8]) {
        let path = self.base.join(root).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn storage(&self) -> LocalStorage {
        LocalStorage::new(self.base.join("lib"))
    }

    async fn scan(&self) {
        self.scan_root("lib").await;
    }

    /// `lib` is the preferred root, so a copy in `other` is the duplicate.
    async fn scan_root(&self, id: &str) {
        let storage = LocalStorage::new(self.base.join(id));
        let root = Root {
            id,
            storage: &storage,
            probe_after_scan: false,
            skip_unchanged: false,
        };
        let order = ["lib".to_owned(), "other".to_owned()];
        scan_root(&self.db, &root, &order, &NoObserver, NOW)
            .await
            .unwrap();
    }

    async fn fill(&self) -> FolderCoverSummary {
        self.fill_root("lib").await
    }

    async fn fill_root(&self, id: &str) -> FolderCoverSummary {
        let storage = LocalStorage::new(self.base.join(id));
        fill_from_folders(&self.db, &storage, &self.dirs, id, &NoObserver, || NOW)
            .await
            .unwrap()
    }

    async fn id(&self, code: &str) -> i64 {
        testing::value(
            &self.db,
            &format!("SELECT id FROM works WHERE code = '{code}'"),
        )
        .await
    }

    async fn cover(&self, code: &str) -> (String, i64) {
        let rows: Vec<(String, i64)> = testing::rows(
            &self.db,
            &format!("SELECT cover_src, cover_version FROM works WHERE code = '{code}'"),
        )
        .await;
        rows.into_iter().next().expect("no such work")
    }

    fn cover_file(&self, id: i64) -> PathBuf {
        self.dirs.covers.join(format!("{id}.jpg"))
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn is_jpeg(path: &Path) -> bool {
    image::guess_format(&fs::read(path).unwrap()).ok() == Some(image::ImageFormat::Jpeg)
}

#[tokio::test]
async fn a_work_without_a_cover_takes_the_image_named_for_it() {
    let lib = Lib::new().await;
    lib.write("RJ01000001/mp3/01.mp3", b"audio");
    lib.write("RJ01000001/图片/封面.png", &png(800, 600));
    lib.write("RJ01000001/图片/音轨列表.png", &png(400, 300));
    lib.write("RJ01000002/01.mp3", b"audio");
    lib.write("RJ01000002/封面.png", &png(100, 100));
    lib.scan().await;
    // Work 2 already has a cover from DLsite, so its folder is not read.
    exec(
        &lib.db,
        "UPDATE works SET cover_src = 'dlsite', cover_version = 4 WHERE code = 'RJ01000002'",
    )
    .await;

    let summary = lib.fill().await;
    assert_eq!(
        summary,
        FolderCoverSummary {
            stored: 1,
            failed: 0,
            unavailable: 0
        }
    );
    assert_eq!(lib.cover("RJ01000001").await, ("folder".to_owned(), 1));
    assert_eq!(lib.cover("RJ01000002").await, ("dlsite".to_owned(), 4));
    let one = lib.id("RJ01000001").await;
    assert!(is_jpeg(&lib.cover_file(one)), "stored as a JPEG");
    let two = lib.id("RJ01000002").await;
    assert!(!lib.cover_file(two).exists());
    assert_eq!(fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);

    // A work with a folder cover is not read again.
    assert_eq!(lib.fill().await, FolderCoverSummary::default());
    assert_eq!(lib.cover("RJ01000001").await, ("folder".to_owned(), 1));
}

#[tokio::test]
async fn a_broken_image_is_not_read_again_and_the_next_one_is_tried() {
    let lib = Lib::new().await;
    lib.write("RJ01000001/01.mp3", b"audio");
    lib.write("RJ01000001/封面.jpg", b"not really a jpeg");
    lib.write("RJ01000001/插图.png", &png(300, 300));
    lib.scan().await;

    let first = lib.fill().await;
    assert_eq!((first.stored, first.failed), (0, 1));
    assert_eq!(lib.cover("RJ01000001").await, ("none".to_owned(), 0));
    assert_eq!(lib.dirs.folder_failures.lock().unwrap().len(), 1);

    // The next scan skips the broken file and takes the illustration.
    let second = lib.fill().await;
    assert_eq!((second.stored, second.failed), (1, 0));
    assert_eq!(lib.cover("RJ01000001").await, ("folder".to_owned(), 1));
}

#[tokio::test]
async fn an_openlist_cover_is_retried_after_download_permission_recovers() {
    for denied in [StatusCode::FORBIDDEN, StatusCode::UNAUTHORIZED] {
        let lib = Lib::new().await;
        let mut jpeg = Vec::new();
        image::load_from_memory(&png(64, 48))
            .unwrap()
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let size = jpeg.len();
        let allowed = Arc::new(AtomicBool::new(false));
        let downloads = Arc::new(AtomicUsize::new(0));
        let links = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let link_base = base.clone();
        let link_count = Arc::clone(&links);
        let download_count = Arc::clone(&downloads);
        let download_allowed = Arc::clone(&allowed);
        let app = Router::new()
            .route(
                "/api/auth/login",
                post(|| async { Json(json!({"code": 200, "data": {"token": "synthetic"}})) }),
            )
            .route(
                "/api/fs/list",
                post(move |Json(body): Json<Value>| async move {
                    let (name, is_dir, size) = match body["path"].as_str().unwrap() {
                        "/library" => ("RJ01000001", true, 0),
                        "/library/RJ01000001" => ("cover.jpg", false, size),
                        path => panic!("unexpected listing: {path}"),
                    };
                    Json(json!({"code": 200, "data": {"content": [{
                        "name": name, "is_dir": is_dir, "size": size,
                        "modified": "2024-05-01T12:00:00Z"
                    }], "total": 1, "provider": "Fake"}}))
                }),
            )
            .route(
                "/api/fs/link",
                post(move |Json(body): Json<Value>| {
                    assert_eq!(body["path"], "/library/RJ01000001/cover.jpg");
                    let serial = link_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let url = format!("{link_base}/cover.jpg?serial={serial}");
                    async move { Json(json!({"code": 200, "data": {"url": url, "header": {}}})) }
                }),
            )
            .route(
                "/cover.jpg",
                get(move || {
                    download_count.fetch_add(1, Ordering::SeqCst);
                    let response = if download_allowed.load(Ordering::SeqCst) {
                        jpeg.clone().into_response()
                    } else {
                        denied.into_response()
                    };
                    async move { response }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut config = openlist_client::Config::new(base.parse().unwrap(), "reader", "synthetic");
        config.link_rate = 1000.0;
        let storage =
            OpenListStorage::new(openlist_client::Client::new(config).unwrap(), "/library")
                .unwrap();
        let root = Root {
            id: "lib",
            storage: &storage,
            probe_after_scan: false,
            skip_unchanged: true,
        };
        scan_root(&lib.db, &root, &["lib".to_owned()], &NoObserver, NOW)
            .await
            .unwrap();
        let before = neo_db::metadata::folder_images(&lib.db, "lib")
            .await
            .unwrap();
        assert_eq!(before.len(), 1);
        let first = fill_from_folders(&lib.db, &storage, &lib.dirs, "lib", &NoObserver, || NOW)
            .await
            .unwrap();
        assert_eq!(
            links.load(Ordering::SeqCst),
            2,
            "the direct link was refreshed"
        );
        assert_eq!(
            downloads.load(Ordering::SeqCst),
            2,
            "both links were refused"
        );
        assert_eq!(lib.cover("RJ01000001").await, ("none".to_owned(), 0));
        assert!(lib.dirs.folder_failures.lock().unwrap().is_empty());
        assert_eq!(
            first,
            FolderCoverSummary {
                stored: 0,
                failed: 0,
                unavailable: 1
            }
        );

        allowed.store(true, Ordering::SeqCst);
        scan_root(&lib.db, &root, &["lib".to_owned()], &NoObserver, NOW)
            .await
            .unwrap();
        let after = neo_db::metadata::folder_images(&lib.db, "lib")
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            (after[0].track_id, after[0].size),
            (before[0].track_id, before[0].size)
        );
        let second = fill_from_folders(&lib.db, &storage, &lib.dirs, "lib", &NoObserver, || NOW)
            .await
            .unwrap();
        assert_eq!(
            second,
            FolderCoverSummary {
                stored: 1,
                failed: 0,
                unavailable: 0
            }
        );
        assert_eq!(downloads.load(Ordering::SeqCst), 3);
        assert_eq!(lib.cover("RJ01000001").await, ("folder".to_owned(), 1));
        let cover = image::open(lib.cover_file(before[0].work_id)).unwrap();
        assert_eq!((cover.width(), cover.height()), (64, 48));
        assert!(is_jpeg(&lib.cover_file(before[0].work_id)));
        server.abort();
    }
}

#[tokio::test]
async fn works_whose_images_say_nothing_keep_the_placeholder() {
    let lib = Lib::new().await;
    lib.write("RJ01000001/01.mp3", b"audio");
    lib.write("RJ01000001/CG/01.png", &png(100, 100));
    lib.write("RJ01000001/ads/RJ01999999_img_main.png", &png(100, 100));
    lib.scan().await;
    assert_eq!(lib.fill().await, FolderCoverSummary::default());
    assert_eq!(lib.cover("RJ01000001").await, ("none".to_owned(), 0));
}

#[tokio::test]
async fn a_cover_that_arrived_meanwhile_is_kept() {
    let lib = Lib::new().await;
    lib.write("RJ01000001/01.mp3", b"audio");
    lib.scan().await;
    let id = lib.id("RJ01000001").await;
    exec(
        &lib.db,
        "UPDATE works SET cover_src = 'dlsite', cover_version = 2",
    )
    .await;
    fs::write(lib.cover_file(id), b"from dlsite").unwrap();

    let stored = store_folder_cover(&lib.db, &lib.dirs, id, &png(10, 10), NOW)
        .await
        .unwrap();
    assert!(!stored);
    assert_eq!(fs::read(lib.cover_file(id)).unwrap(), b"from dlsite");
    assert_eq!(lib.cover("RJ01000001").await, ("dlsite".to_owned(), 2));
    assert_eq!(fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);
}

/// Records the stages the folder pass reports.
#[derive(Default)]
struct Stages(Mutex<Vec<(Stage, u64, u64)>>);

impl Observer for Stages {
    fn progress(&self, stage: Stage, done: u64, total: u64) {
        self.0.lock().unwrap().push((stage, done, total));
    }
}

#[tokio::test]
async fn progress_counts_the_works_that_have_a_candidate() {
    let lib = Lib::new().await;
    for code in ["RJ01000001", "RJ01000002"] {
        lib.write(&format!("{code}/01.mp3"), b"audio");
        lib.write(&format!("{code}/cover.png"), &png(50, 50));
    }
    lib.write("RJ01000003/01.mp3", b"audio");
    lib.scan().await;
    let stages = Stages::default();
    fill_from_folders(&lib.db, &lib.storage(), &lib.dirs, "lib", &stages, || NOW)
        .await
        .unwrap();
    assert_eq!(
        *stages.0.lock().unwrap(),
        [
            (Stage::Covers, 0, 2),
            (Stage::Covers, 1, 2),
            (Stage::Covers, 2, 2)
        ]
    );
}

#[tokio::test]
async fn the_copy_in_another_root_is_read_when_that_root_is_scanned() {
    let lib = Lib::new().await;
    let cover = png(64, 64);
    for root in ["lib", "other"] {
        lib.write_in(root, "RJ01000001/01.mp3", b"audio");
        lib.write_in(root, "RJ01000001/封面.png", &cover);
    }
    lib.scan_root("lib").await;
    lib.scan_root("other").await;
    let duplicates: i64 = testing::value(
        &lib.db,
        "SELECT COUNT(*) FROM tracks WHERE kind = 'image' AND duplicate_of IS NOT NULL",
    )
    .await;
    assert_eq!(duplicates, 1, "the copy in other is the duplicate");

    // `lib` may be the root that cannot be read, so `other` uses its copy.
    assert_eq!(lib.fill_root("other").await.stored, 1);
    assert_eq!(lib.cover("RJ01000001").await, ("folder".to_owned(), 1));
}

#[tokio::test]
async fn a_covers_directory_that_refuses_the_cover_stops_the_stage() {
    use std::os::unix::fs::PermissionsExt;

    let lib = Lib::new().await;
    for code in ["RJ01000001", "RJ01000002"] {
        lib.write(&format!("{code}/01.mp3"), b"audio");
        lib.write(&format!("{code}/封面.png"), &png(40, 40));
    }
    lib.scan().await;
    // The lock file exists, so only publishing the cover meets the refusal.
    fs::write(lib.dirs.covers.join(".lock"), b"").unwrap();
    let mode = fs::Permissions::from_mode;
    fs::set_permissions(&lib.dirs.covers, mode(0o500)).unwrap();
    let summary = lib.fill().await;
    fs::set_permissions(&lib.dirs.covers, mode(0o700)).unwrap();

    assert_eq!(
        summary,
        FolderCoverSummary {
            stored: 0,
            failed: 0,
            unavailable: 2
        },
        "the second work is left for the next scan, not read in vain"
    );
    assert!(lib.dirs.folder_failures.lock().unwrap().is_empty());
    assert_eq!(lib.cover("RJ01000001").await, ("none".to_owned(), 0));
    assert_eq!(fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);
}
