//! The DLsite provider against a local fake DLsite, and the storage rules
//! for fetched metadata. Nothing here touches the network.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use neo_db::{
    Db,
    testing::{self, TestDb, exec},
};
use neo_scanner::{
    NoObserver,
    metadata::{
        AsmrOneConfig, AsmrOneProvider, CoverDirs, DlsiteConfig, DlsiteProvider, FetchOutcome,
        HvdbConfig, HvdbProvider, MetadataError, MetadataProvider, ProviderChain, fetch_and_store,
        fetch_missing,
    },
};
use serde_json::{Value, json};
use tokio::time::Instant;

#[path = "support/commit_proxy.rs"]
mod commit_proxy;

#[derive(Default)]
struct Fake {
    /// Requests per path, with the time they arrived.
    log: Mutex<Vec<(String, Instant)>>,
    /// How many more times each code answers 500 before it works.
    failures: Mutex<HashMap<String, u32>>,
    /// Covers that redirect to a host the provider may not contact.
    redirects: Mutex<Vec<String>>,
    /// Covers whose image requests answer 404.
    missing_covers: Mutex<Vec<String>>,
}

/// The circle left the voice actors out for RJ01000005.
fn voice_by(code: &str) -> Value {
    if code == "RJ01000005" {
        return json!({});
    }
    json!({"voice_by": [{"name": "Voice A"}, {"name": "Voice B"}]})
}

fn product(code: &str, base: &str) -> Value {
    json!([{
        "workno": code,
        "product_name": format!("Title of {code} [Circle] | DLsite"),
        "maker_id": "RG12345",
        "maker_name": "Circle Name",
        "age_category": 3,
        "regist_date": "2024-05-01 16:00:00",
        "genres": [{"id": 497, "name": "ASMR"}, {"id": 9001, "name": "New Genre"}],
        "creaters": voice_by(code),
        "image_main": {"url": format!("{base}img/{code}.jpg")},
    }])
}

async fn product_json(
    State((fake, base)): State<(Arc<Fake>, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let code = q.get("workno").cloned().unwrap_or_default();
    fake.log
        .lock()
        .unwrap()
        .push((format!("product {code}"), Instant::now()));
    if fake
        .redirects
        .lock()
        .unwrap()
        .iter()
        .any(|r| r == "product")
    {
        return axum::response::Redirect::temporary("http://maintenance.example.invalid/")
            .into_response();
    }
    if let Some(left) = fake.failures.lock().unwrap().get_mut(&code)
        && *left > 0
    {
        *left -= 1;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if code == "RJ404404" {
        return Json(json!([])).into_response();
    }
    Json(product(&code, &base)).into_response()
}

async fn info(Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let code = q.get("product_id").cloned().unwrap_or_default();
    Json(json!({ code: {
        "dl_count": "1,234",
        "rate_average_2dp": 4.62,
        "rate_count": 318,
        "rate_count_detail": [{"review_point": 5, "count": 300, "ratio": 94}],
        "rank": [],
        "price": 1320,
    }}))
}

async fn image(
    State((fake, _)): State<(Arc<Fake>, String)>,
    axum::extract::Path(file): axum::extract::Path<String>,
) -> Response {
    fake.log
        .lock()
        .unwrap()
        .push((format!("img {file}"), Instant::now()));
    if fake.missing_covers.lock().unwrap().contains(&file) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if fake.redirects.lock().unwrap().contains(&file) {
        return axum::response::Redirect::to("http://cover.example.invalid/elsewhere.jpg")
            .into_response();
    }
    if let Some(left) = fake
        .failures
        .lock()
        .unwrap()
        .get_mut(&format!("img {file}"))
        && *left > 0
    {
        *left -= 1;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    (&b"jpeg bytes"[..]).into_response()
}

/// Starts the fake on a free port and returns a provider pointed at it.
async fn fake_dlsite(spacing: Duration) -> (Arc<Fake>, DlsiteProvider) {
    let fake = Arc::new(Fake::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/maniax/api/=/product.json", get(product_json))
        .route("/maniax/product/info/ajax", get(info))
        .route("/img/{file}", get(image))
        .with_state((fake.clone(), base.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = DlsiteProvider::new(DlsiteConfig {
        base: base.parse().unwrap(),
        spacing,
        retries: 3,
        backoff: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..DlsiteConfig::default()
    })
    .unwrap();
    (fake, provider)
}

/// title, metadata_src, cover_src, circle_id, release_date, nsfw, dl_count,
/// rate_average, extra, cover_version
type WorkRow = (
    String,
    String,
    String,
    Option<i64>,
    Option<String>,
    bool,
    Option<i64>,
    Option<f64>,
    String,
    i64,
);

struct Lib {
    _dir: tempfile::TempDir,
    _db: TestDb,
    db: Db,
    dirs: CoverDirs,
}

impl Lib {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let t = testing::open().await;
        let db = t.db.clone();
        let covers: PathBuf = dir.path().join("covers");
        let tmp: PathBuf = dir.path().join("tmp");
        std::fs::create_dir_all(&covers).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        // Genre 497 already exists under another name, with "ASMR" as its
        // alias, as an admin merge leaves it.
        exec(
            &db,
            r#"INSERT INTO tags (id, name) VALUES (497, '助眠');
               INSERT INTO tag_aliases (alias, tag_id) VALUES ('助眠', 497), ('ASMR', 497);
               INSERT INTO roots (id, display_name) VALUES ('lib', 'lib');
               INSERT INTO works (code, title, metadata_src, cover_src, extra, created_at, updated_at)
               VALUES ('RJ01000001', 'dir name', 'none', 'none',
                       '{"legacy_durations":{"01.mp3":1}}', 0, 0),
                      ('RJ01000002', 'hand made', 'manual', 'imported', '{}', 0, 0),
                      ('RJ404404', 'unknown', 'none', 'none', '{}', 0, 0);"#,
        )
        .await;
        std::fs::write(covers.join("2.jpg"), b"imported cover").unwrap();
        Self {
            _dir: dir,
            _db: t,
            db,
            dirs: CoverDirs::new(covers, tmp),
        }
    }

    async fn work(&self, id: i64) -> Value {
        let row: WorkRow = testing::rows(
            &self.db,
            &format!(
                "SELECT title, metadata_src, cover_src, circle_id, release_date, nsfw, dl_count,
                        rate_average, extra, cover_version FROM works WHERE id = {id}"
            ),
        )
        .await
        .into_iter()
        .next()
        .expect("no such work");
        json!({
            "title": row.0, "metadata_src": row.1, "cover_src": row.2, "circle_id": row.3,
            "release_date": row.4, "nsfw": row.5, "dl_count": row.6, "rate_average": row.7,
            "extra": serde_json::from_str::<Value>(&row.8).unwrap(), "cover_version": row.9,
        })
    }
}

#[tokio::test]
async fn new_works_get_metadata_and_a_cover() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(1).await;
    assert_eq!(w["title"], "Title of RJ01000001");
    assert_eq!(w["metadata_src"], "dlsite");
    assert_eq!(w["circle_id"], 12345);
    assert_eq!(w["release_date"], "2024-05-01");
    assert_eq!(w["nsfw"], true);
    assert_eq!(w["dl_count"], 1234);
    assert_eq!(w["rate_average"], 4.62);
    assert_eq!(
        w["extra"]["legacy_durations"]["01.mp3"], 1,
        "other keys stay"
    );
    assert!(
        w["extra"].get("rank").is_none(),
        "an empty rank is not stored"
    );
    assert_eq!(w["cover_src"], "dlsite");
    assert_eq!(w["cover_version"], 1);
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("1.jpg")).unwrap(),
        b"jpeg bytes"
    );
    assert_eq!(std::fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);

    let vas: Vec<(String, String)> = testing::rows(
        &lib.db,
        "SELECT v.id, v.name FROM work_vas wv JOIN vas v ON v.id = wv.va_id
         WHERE wv.work_id = 1 ORDER BY v.name",
    )
    .await;
    // The old server's name-based ID, so imported voice actors match.
    assert_eq!(
        vas[0],
        (
            "87bb655b-88ab-5ab8-960d-8fd65d83f70d".into(),
            "Voice A".into()
        )
    );
    assert_eq!(vas.len(), 2);
    let tag_ids: Vec<i64> = testing::column(
        &lib.db,
        "SELECT tag_id FROM work_tags WHERE work_id = 1 ORDER BY tag_id",
    )
    .await;
    assert_eq!(
        tag_ids,
        [497, 9001],
        "aliases first, then the DLsite genre ID"
    );
}

#[tokio::test]
async fn manual_metadata_and_imported_covers_need_force() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 2, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::ManualKept);
    let w = lib.work(2).await;
    assert_eq!(
        (w["title"].as_str(), w["metadata_src"].as_str()),
        (Some("hand made"), Some("manual"))
    );
    assert_eq!(w["cover_src"], "imported");
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"imported cover"
    );

    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 2, true, 200)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(2).await;
    assert_eq!(w["metadata_src"], "dlsite");
    assert_eq!(w["cover_src"], "dlsite");
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"jpeg bytes"
    );
}

#[tokio::test]
async fn a_folder_cover_gives_way_to_the_dlsite_cover() {
    let lib = Lib::new().await;
    exec(
        &lib.db,
        "UPDATE works SET cover_src = 'folder', cover_version = 1 WHERE id = 1",
    )
    .await;
    std::fs::write(lib.dirs.covers.join("1.jpg"), b"folder cover").unwrap();
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(1).await;
    assert_eq!(
        (w["cover_src"].as_str(), w["cover_version"].as_i64()),
        (Some("dlsite"), Some(2))
    );
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("1.jpg")).unwrap(),
        b"jpeg bytes"
    );
}

#[tokio::test]
async fn a_failed_commit_keeps_the_previous_cover() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    // Fails the write that publishes a cover, after the file was renamed.
    let trigger = if testing::on_postgres() {
        "CREATE FUNCTION fail_cover() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'injected'; END $$;
         CREATE TRIGGER fail_cover BEFORE UPDATE OF cover_version ON works
           FOR EACH ROW EXECUTE FUNCTION fail_cover();"
    } else {
        "CREATE TRIGGER fail_cover BEFORE UPDATE OF cover_version ON works
         BEGIN SELECT RAISE(ABORT, 'injected'); END;"
    };
    exec(&lib.db, trigger).await;

    assert!(
        fetch_and_store(&lib.db, &provider, &lib.dirs, 2, true, 100)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"imported cover"
    );
    assert!(
        fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
            .await
            .is_err()
    );
    assert!(!lib.dirs.covers.join("1.jpg").exists());
    assert_eq!(std::fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);
    let w = lib.work(2).await;
    assert_eq!(
        (w["cover_src"].as_str(), w["cover_version"].as_i64()),
        (Some("imported"), Some(0))
    );
}

#[tokio::test]
async fn an_unsettled_cover_leaves_metadata_for_the_scan_after_recovery() {
    use neo_scanner::metadata::recover_cover_swaps;

    let lib = Lib::new().await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    let original = lib.work(1).await;
    exec(
        &lib.db,
        "INSERT INTO work_locations (work_id, root_id, rel_path, display_path) VALUES
           (1, 'lib', CAST('1' AS BLOB), 'a'), (3, 'lib', CAST('3' AS BLOB), 'c');
         CREATE TABLE cover_commit_failure (
           work_id BIGINT REFERENCES works(id) DEFERRABLE INITIALLY DEFERRED
         );",
    )
    .await;
    // The deferred foreign key fails COMMIT, after the new cover is published.
    let trigger = if testing::on_postgres() {
        "CREATE FUNCTION fail_cover_commit() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN INSERT INTO cover_commit_failure VALUES (-1); RETURN NEW; END $$;
         CREATE TRIGGER fail_cover_commit AFTER UPDATE OF cover_version ON works
           FOR EACH ROW EXECUTE FUNCTION fail_cover_commit();"
    } else {
        "CREATE TRIGGER fail_cover_commit AFTER UPDATE OF cover_version ON works
         BEGIN INSERT INTO cover_commit_failure VALUES (-1); END;"
    };
    exec(&lib.db, trigger).await;
    let error = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
        .await
        .unwrap_err();
    assert!(
        error
            .as_database_error()
            .is_some_and(|e| e.is_foreign_key_violation()),
        "{error}"
    );
    assert_eq!(lib.work(1).await, original);
    let target = lib.dirs.covers.join("1.jpg");
    assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");
    assert!(lib.dirs.lock.read().await.contains(&1));

    // Storage works again, but only startup can settle the earlier swap.
    exec(
        &lib.db,
        if testing::on_postgres() {
            "DROP TRIGGER fail_cover_commit ON works"
        } else {
            "DROP TRIGGER fail_cover_commit"
        },
    )
    .await;
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 200)
        .await
        .unwrap();
    assert_eq!(lib.work(1).await, original);
    assert!(matches!(outcome, FetchOutcome::CoverNotStored(_)));
    let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 300)
        .await
        .unwrap();
    assert_eq!(
        (summary.fetched, summary.not_found, summary.unavailable),
        (0, 0, 2)
    );
    assert_eq!(lib.work(1).await, original);
    assert!(
        fake.log
            .lock()
            .unwrap()
            .iter()
            .all(|(path, _)| path != "product RJ404404"),
        "the unsettled cover must stop the automatic metadata stage"
    );
    assert_eq!(std::fs::read_dir(&lib.dirs.tmp).unwrap().count(), 0);

    let restarted = CoverDirs::new(lib.dirs.covers.clone(), lib.dirs.tmp.clone());
    assert_eq!(recover_cover_swaps(&lib.db, &restarted).await.unwrap(), 1);
    assert!(!target.exists());
    let summary = fetch_missing(&lib.db, &provider, &restarted, "lib", &NoObserver, || 400)
        .await
        .unwrap();
    assert_eq!(
        (summary.fetched, summary.not_found, summary.unavailable),
        (1, 1, 0)
    );
    let work = lib.work(1).await;
    assert_eq!(work["metadata_src"], "dlsite");
    assert_eq!(work["cover_src"], "dlsite");
    assert_eq!(work["cover_version"], 1);
    assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");
    assert_eq!(std::fs::read_dir(restarted.swap_dir()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&restarted.tmp).unwrap().count(), 0);
}

#[tokio::test]
async fn an_unsettled_cover_stays_retryable_when_the_next_cover_is_missing() {
    if !testing::on_postgres() {
        return;
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        use commit_proxy::{CommitProxy, Disconnect};
        use neo_scanner::metadata::recover_cover_swaps;

        let lib = Lib::new().await;
        let (fake, provider) = fake_dlsite(Duration::ZERO).await;
        exec(
            &lib.db,
            "INSERT INTO work_locations (work_id, root_id, rel_path, display_path)
             VALUES (1, 'lib', CAST('1' AS BLOB), 'a');",
        )
        .await;
        let original = lib.work(1).await;
        let mut proxy = CommitProxy::start(&lib.db, Disconnect::BeforeCommit).await;
        assert!(
            fetch_and_store(&proxy.db, &provider, &lib.dirs, 1, false, 100)
                .await
                .is_err()
        );
        proxy.assert_disconnected().await;
        assert_eq!(lib.work(1).await, original);
        let target = lib.dirs.covers.join("1.jpg");
        assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");

        fake.missing_covers
            .lock()
            .unwrap()
            .push("RJ01000001.jpg".to_owned());
        let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 200)
            .await
            .unwrap();
        assert!(
            matches!(outcome, FetchOutcome::CoverNotStored(_)),
            "{outcome:?}"
        );
        assert_eq!(lib.work(1).await, original);
        let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 300)
            .await
            .unwrap();
        assert_eq!((summary.fetched, summary.unavailable), (0, 1));
        assert_eq!(lib.work(1).await["metadata_src"], "none");

        let restarted = CoverDirs::new(lib.dirs.covers.clone(), lib.dirs.tmp.clone());
        assert_eq!(recover_cover_swaps(&lib.db, &restarted).await.unwrap(), 1);
        assert!(!target.exists());
        fake.missing_covers.lock().unwrap().clear();
        let summary = fetch_missing(&lib.db, &provider, &restarted, "lib", &NoObserver, || 400)
            .await
            .unwrap();
        assert_eq!(
            (summary.fetched, summary.not_found, summary.unavailable),
            (1, 0, 0)
        );
        let work = lib.work(1).await;
        assert_eq!(work["metadata_src"], "dlsite");
        assert_eq!(work["cover_src"], "dlsite");
        assert_eq!(work["cover_version"], 1);
        assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");
        assert_eq!(std::fs::read_dir(restarted.swap_dir()).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(&restarted.tmp).unwrap().count(), 0);
    })
    .await
    .expect("the PostgreSQL cover retry test timed out");
}

async fn disconnected_cover_commit(replacing: bool, committed: bool) {
    if !testing::on_postgres() {
        return;
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        use commit_proxy::{CommitProxy, Disconnect};
        use neo_scanner::metadata::{recover_cover_swaps, store_folder_cover};

        let lib = Lib::new().await;
        let mut proxy = CommitProxy::start(
            &lib.db,
            if committed {
                Disconnect::AfterCommit
            } else {
                Disconnect::BeforeCommit
            },
        )
        .await;
        let id = if replacing { 2 } else { 1 };
        let target = lib.dirs.covers.join(format!("{id}.jpg"));
        if replacing {
            let (_, provider) = fake_dlsite(Duration::ZERO).await;
            assert!(
                fetch_and_store(&proxy.db, &provider, &lib.dirs, id, true, 100)
                    .await
                    .is_err()
            );
        } else {
            // No metadata provider is involved, as with metadata.source = "none".
            assert!(
                store_folder_cover(&proxy.db, &lib.dirs, id, b"jpeg bytes", 100)
                    .await
                    .is_err()
            );
        }
        proxy.assert_disconnected().await;
        let expected_source = match (replacing, committed) {
            (true, true) => "dlsite",
            (false, true) => "folder",
            (true, false) => "imported",
            (false, false) => "none",
        };
        let work = lib.work(id).await;
        assert_eq!(work["cover_version"], i64::from(committed));
        assert_eq!(work["cover_src"], expected_source);
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"jpeg bytes",
            "a lost reply must not undo the file swap"
        );
        assert!(
            lib.dirs.lock.read().await.contains(&id),
            "readers must not cache an unconfirmed cover"
        );
        assert!(
            std::fs::read_dir(lib.dirs.swap_dir())
                .unwrap()
                .any(|entry| {
                    entry
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "json")
                }),
            "recovery needs the journal even though commit() failed"
        );

        // A later refresh must not overwrite the journal's version or file.
        let (_, provider) = fake_dlsite(Duration::ZERO).await;
        fetch_and_store(&lib.db, &provider, &lib.dirs, id, true, 200)
            .await
            .unwrap();
        assert_eq!(lib.work(id).await["cover_version"], i64::from(committed));
        assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");

        // Startup creates fresh locks and settles files against the stored version.
        let restarted = CoverDirs::new(lib.dirs.covers.clone(), lib.dirs.tmp.clone());
        assert_eq!(recover_cover_swaps(&lib.db, &restarted).await.unwrap(), 1);
        if committed {
            assert_eq!(std::fs::read(&target).unwrap(), b"jpeg bytes");
        } else if replacing {
            assert_eq!(std::fs::read(&target).unwrap(), b"imported cover");
        } else {
            assert!(!target.exists());
        }
        assert_eq!(std::fs::read_dir(restarted.swap_dir()).unwrap().count(), 0);
        assert_eq!(recover_cover_swaps(&lib.db, &restarted).await.unwrap(), 0);
    })
    .await
    .expect("the PostgreSQL commit fault test timed out");
}

#[tokio::test]
async fn a_lost_commit_reply_preserves_the_first_folder_cover() {
    disconnected_cover_commit(false, true).await;
}

#[tokio::test]
async fn a_lost_commit_reply_preserves_a_replaced_cover() {
    disconnected_cover_commit(true, true).await;
}

#[tokio::test]
async fn a_disconnect_before_commit_recovers_the_first_folder_cover() {
    disconnected_cover_commit(false, false).await;
}

#[tokio::test]
async fn a_disconnect_before_commit_recovers_a_replaced_cover() {
    disconnected_cover_commit(true, false).await;
}

#[tokio::test]
async fn a_cover_is_not_replaced_while_a_reader_holds_the_lock() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    // A cover request holds the read side while it pairs cover_version
    // with the file.
    let reading = lib.dirs.lock.clone().read_owned().await;
    let store = {
        let (db, dirs) = (lib.db.clone(), lib.dirs.clone());
        tokio::spawn(async move { fetch_and_store(&db, &provider, &dirs, 2, true, 100).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"imported cover"
    );
    drop(reading);
    assert_eq!(store.await.unwrap().unwrap(), FetchOutcome::Updated);
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"jpeg bytes"
    );
}

#[tokio::test]
async fn a_scrape_does_not_rename_a_circle_a_manual_work_uses() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    exec(
        &lib.db,
        "INSERT INTO circles (id, name) VALUES (12345, 'Hand Named');
         UPDATE works SET circle_id = 12345 WHERE id = 2;",
    )
    .await;
    fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    let name: String = testing::value(&lib.db, "SELECT name FROM circles WHERE id = 12345").await;
    assert_eq!(name, "Hand Named");
    assert_eq!(lib.work(1).await["circle_id"], 12345);

    // Forcing the manual work itself replaces its own circle's name.
    exec(&lib.db, "UPDATE works SET circle_id = NULL WHERE id = 1").await;
    fetch_and_store(&lib.db, &provider, &lib.dirs, 2, true, 200)
        .await
        .unwrap();
    let name: String = testing::value(&lib.db, "SELECT name FROM circles WHERE id = 12345").await;
    assert_ne!(name, "Hand Named");
}

#[tokio::test]
async fn a_cover_is_not_replaced_while_a_backup_runs() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    let backup = neo_scanner::metadata::lock_covers(&lib.dirs.covers).unwrap();
    let store = {
        let (db, dirs) = (lib.db.clone(), lib.dirs.clone());
        tokio::spawn(async move { fetch_and_store(&db, &provider, &dirs, 2, true, 100).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"imported cover"
    );
    drop(backup);
    assert_eq!(store.await.unwrap().unwrap(), FetchOutcome::Updated);
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"jpeg bytes"
    );
}

#[tokio::test]
async fn a_refresh_cancelled_while_a_backup_runs_writes_nothing() {
    let lib = Lib::new().await;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    let backup = neo_scanner::metadata::lock_covers(&lib.dirs.covers).unwrap();
    let store = {
        let (db, dirs) = (lib.db.clone(), lib.dirs.clone());
        tokio::spawn(async move { fetch_and_store(&db, &provider, &dirs, 2, true, 100).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    store.abort();
    drop(backup);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let w = lib.work(2).await;
    assert_eq!(w["metadata_src"], "manual");
    assert_eq!(
        std::fs::read_dir(&lib.dirs.tmp).unwrap().count(),
        0,
        "the downloaded cover is removed"
    );
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("2.jpg")).unwrap(),
        b"imported cover"
    );
}

#[tokio::test]
async fn a_cover_outage_leaves_the_work_for_the_next_scan() {
    let lib = Lib::new().await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    fake.failures
        .lock()
        .unwrap()
        .insert("img RJ01000001.jpg".into(), 10);
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Unavailable(_)),
        "{outcome:?}"
    );
    let w = lib.work(1).await;
    assert_eq!(w["metadata_src"], "none", "retried by the next scan");
    assert_eq!(w["cover_src"], "none");
}

#[tokio::test]
async fn unknown_works_and_outages_leave_the_work_as_it_was() {
    let lib = Lib::new().await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 3, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::NotFound);
    assert_eq!(lib.work(3).await["metadata_src"], "none");

    // Two failures are retried away.
    fake.failures.lock().unwrap().insert("RJ01000001".into(), 2);
    assert!(provider.fetch("RJ01000001").await.is_ok());
    // Five are not: the first attempt and three retries all fail.
    fake.failures.lock().unwrap().insert("RJ01000001".into(), 5);
    assert!(matches!(
        provider.fetch("RJ01000001").await,
        Err(MetadataError::Unavailable(_))
    ));
    let attempts = fake
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p == "product RJ01000001")
        .count();
    assert_eq!(attempts, 3 + 4);
}

#[tokio::test]
async fn the_scan_stage_stops_at_the_first_outage() {
    let lib = Lib::new().await;
    exec(
        &lib.db,
        "INSERT INTO work_locations (work_id, root_id, rel_path, display_path) VALUES
           (1, 'lib', CAST('1' AS BLOB), 'a'), (3, 'lib', CAST('3' AS BLOB), 'c');",
    )
    .await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    fake.failures
        .lock()
        .unwrap()
        .insert("RJ01000001".into(), 100);
    let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 100)
        .await
        .unwrap();
    assert_eq!((summary.fetched, summary.unavailable), (0, 2));
    assert_eq!(lib.work(1).await["metadata_src"], "none");

    // With the network back, the next run fetches what is still missing.
    fake.failures.lock().unwrap().clear();
    let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 100)
        .await
        .unwrap();
    assert_eq!((summary.fetched, summary.not_found), (1, 1));
}

#[tokio::test]
async fn a_cover_that_cannot_be_written_leaves_the_work_for_the_next_scan() {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::Permissions::from_mode;
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    // Once with the temporary directory refusing the download, once with
    // the covers directory refusing the rename.
    for refusing in ["tmp", "covers"] {
        let lib = Lib::new().await;
        // The lock file exists, so only writing the cover meets the refusal.
        std::fs::write(lib.dirs.covers.join(".lock"), b"").unwrap();
        let dir = if refusing == "tmp" {
            &lib.dirs.tmp
        } else {
            &lib.dirs.covers
        };
        std::fs::set_permissions(dir, mode(0o500)).unwrap();
        let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100).await;
        std::fs::set_permissions(dir, mode(0o700)).unwrap();
        assert!(
            matches!(outcome, Ok(FetchOutcome::CoverNotStored(_))),
            "{refusing}: {outcome:?}"
        );
        // Nothing was written, so the work is still one a scan fetches.
        let w = lib.work(1).await;
        assert_eq!(
            (w["metadata_src"].as_str(), w["cover_src"].as_str()),
            (Some("none"), Some("none")),
            "{refusing}"
        );
        let outcome = fetch_and_store(&lib.db, &provider, &lib.dirs, 1, false, 100)
            .await
            .unwrap();
        assert_eq!(outcome, FetchOutcome::Updated, "{refusing}");
        assert_eq!(lib.work(1).await["cover_src"], "dlsite", "{refusing}");
    }
}

#[tokio::test]
async fn a_cover_redirected_off_dlsite_is_no_cover_rather_than_an_outage() {
    let lib = Lib::new().await;
    exec(
        &lib.db,
        "INSERT INTO works (code, title, metadata_src, cover_src, created_at, updated_at)
         VALUES ('RJ01000004', 'next', 'none', 'none', 0, 0);
         INSERT INTO work_locations (work_id, root_id, rel_path, display_path) VALUES
           (1, 'lib', CAST('1' AS BLOB), 'a'),
           ((SELECT id FROM works WHERE code = 'RJ01000004'), 'lib', CAST('4' AS BLOB), 'd');",
    )
    .await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    fake.redirects.lock().unwrap().push("RJ01000001.jpg".into());
    let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 100)
        .await
        .unwrap();
    // The metadata is kept without a cover, and the next work is fetched.
    assert_eq!((summary.fetched, summary.unavailable), (2, 0));
    let w = lib.work(1).await;
    assert_eq!(
        (w["metadata_src"].as_str(), w["cover_src"].as_str()),
        (Some("dlsite"), Some("none"))
    );
    let asked = fake
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p == "img RJ01000001.jpg")
        .count();
    assert_eq!(asked, 1, "asking again gets the same redirect");
}

#[tokio::test]
async fn the_api_redirected_off_dlsite_is_an_outage() {
    let lib = Lib::new().await;
    exec(
        &lib.db,
        "INSERT INTO work_locations (work_id, root_id, rel_path, display_path) VALUES
           (1, 'lib', CAST('1' AS BLOB), 'a'), (3, 'lib', CAST('3' AS BLOB), 'c');",
    )
    .await;
    let (fake, provider) = fake_dlsite(Duration::ZERO).await;
    fake.redirects.lock().unwrap().push("product".into());
    let summary = fetch_missing(&lib.db, &provider, &lib.dirs, "lib", &NoObserver, || 100)
        .await
        .unwrap();
    // Not "not found" for every work: the stage stops at the first one.
    assert_eq!(
        (summary.fetched, summary.not_found, summary.unavailable),
        (0, 0, 2)
    );
}

#[tokio::test]
async fn covers_outside_dlsite_are_refused() {
    let (_, provider) = fake_dlsite(Duration::ZERO).await;
    for url in [
        "http://169.254.169.254/latest/meta-data",
        "file:///etc/passwd",
    ] {
        // Not an outage: retrying would not help, and the work keeps its
        // metadata without a cover.
        assert_eq!(
            provider.cover(&url.parse().unwrap()).await,
            Err(MetadataError::NotFound)
        );
    }
}

/// Answers with good headers and a body that breaks off, `broken` times,
/// then with a whole answer.
async fn truncating_server(broken: u32) -> (Arc<Mutex<u32>>, DlsiteProvider) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let served = Arc::new(Mutex::new(0u32));
    let count = served.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let n = {
                let mut c = count.lock().unwrap();
                *c += 1;
                *c
            };
            let body = r#"[{"product_name":"Whole"}]"#;
            if n <= broken {
                let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 500\r\n\r\n[{\"prod";
                let _ = socket.write_all(head.as_bytes()).await;
            } else {
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
            // Closing mid-body is the failure being simulated.
            drop(socket);
        }
    });
    let provider = DlsiteProvider::new(DlsiteConfig {
        base: base.parse().unwrap(),
        spacing: Duration::ZERO,
        retries: 3,
        backoff: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..DlsiteConfig::default()
    })
    .unwrap();
    (served, provider)
}

#[tokio::test]
async fn a_body_that_breaks_off_is_retried() {
    let (served, provider) = truncating_server(2).await;
    let meta = provider.fetch("RJ01000001").await.unwrap();
    assert_eq!(meta.title, "Whole");
    // Two broken product answers, the good one, then the info request.
    assert!(*served.lock().unwrap() >= 3);
}

#[test]
fn shutdown_does_not_wait_for_a_refresh_queued_behind_a_backup() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (lib, provider) =
        rt.block_on(async { (Lib::new().await, fake_dlsite(Duration::ZERO).await.1) });
    let backup = neo_scanner::metadata::lock_covers(&lib.dirs.covers).unwrap();
    let (db, dirs) = (lib.db.clone(), lib.dirs.clone());
    rt.spawn(async move {
        let _ = fetch_and_store(&db, &provider, &dirs, 2, true, 100).await;
    });
    std::thread::sleep(Duration::from_millis(300));
    let (done, stopped) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(rt);
        let _ = done.send(());
    });
    // A runtime that waits for the refresh never stops while the backup
    // holds the lock, so the bound only has to tell a hang from a slow
    // shutdown on a loaded machine.
    assert!(stopped.recv_timeout(Duration::from_secs(60)).is_ok());
    drop(backup);
    // The pools belonged to the runtime that is gone.
    std::mem::forget(lib);
}

/// A fake asmr.one mirror: `workInfo` for the numbers in `known`, 404 for
/// the rest (including the `workInfo/1` probe, as the real one answers),
/// and covers. `down` makes every request answer 500.
struct FakeAsmr {
    known: Vec<u64>,
    down: bool,
    log: Mutex<Vec<String>>,
}

async fn asmr_mirror(known: &[u64], down: bool) -> (Arc<FakeAsmr>, url::Url) {
    let fake = Arc::new(FakeAsmr {
        known: known.to_vec(),
        down,
        log: Mutex::new(Vec::new()),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let handle = {
        let fake = fake.clone();
        let base = base.clone();
        move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
            let fake = fake.clone();
            let base = base.clone();
            async move {
                let path = uri.path().to_owned();
                fake.log.lock().unwrap().push(path.clone());
                assert_eq!(
                    headers.get("cookie").and_then(|v| v.to_str().ok()),
                    Some("locale=zh-cn")
                );
                if fake.down {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                if let Some(n) = path.strip_prefix("/api/workInfo/") {
                    let n: u64 = n.parse().unwrap_or(0);
                    if !fake.known.contains(&n) {
                        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
                            .into_response();
                    }
                    return Json(json!({
                        "id": n,
                        "title": format!("asmr title {n}"),
                        "circle_id": 54321,
                        "name": "asmr circle",
                        "nsfw": true,
                        "release": "2023-02-03",
                        "dl_count": 7,
                        "price": 990,
                        "rate_average_2dp": 0,
                        "rate_count": 0,
                        "rate_count_detail": [],
                        "rank": null,
                        "vas": [{"id": "x", "name": "Voice C"}],
                        "tags": [{"id": 1, "name": "fallback", "i18n": {"zh-cn": {"name": "中文标签"}}}],
                        "circle": {"id": 54321, "name": "asmr circle", "source_id": "RG54321"},
                        "mainCoverUrl": format!("{base}api/cover/{n}.jpg?type=main"),
                    }))
                    .into_response();
                }
                if path.starts_with("/api/cover/") {
                    return (&b"asmr jpeg"[..]).into_response();
                }
                StatusCode::NOT_FOUND.into_response()
            }
        }
    };
    let app = Router::new().fallback(handle);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, base.parse().unwrap())
}

fn asmr_provider(mirrors: Vec<url::Url>) -> AsmrOneProvider {
    AsmrOneProvider::new(AsmrOneConfig {
        mirrors,
        spacing: Duration::ZERO,
        backoff: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        probe_timeout: Duration::from_secs(2),
        ..AsmrOneConfig::default()
    })
    .unwrap()
}

/// DLsite first, the fake asmr.one second.
async fn chain(known: &[u64]) -> (Arc<Fake>, Arc<FakeAsmr>, ProviderChain) {
    let (dlsite_fake, dlsite) = fake_dlsite(Duration::ZERO).await;
    let (asmr_fake, mirror) = asmr_mirror(known, false).await;
    let chain = ProviderChain::new(
        Arc::new(dlsite),
        Some(Arc::new(asmr_provider(vec![mirror]))),
    );
    (dlsite_fake, asmr_fake, chain)
}

#[tokio::test]
async fn a_work_dlsite_does_not_have_comes_from_asmr_one() {
    let lib = Lib::new().await;
    let (_, _, chain) = chain(&[404404]).await;
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 3, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(3).await;
    assert_eq!(w["title"], "asmr title 404404");
    assert_eq!(w["metadata_src"], "asmr_one");
    assert_eq!(w["circle_id"], 54321, "the RG code, like DLsite's");
    assert_eq!(w["release_date"], "2023-02-03");
    assert_eq!(w["cover_src"], "asmr_one");
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("3.jpg")).unwrap(),
        b"asmr jpeg"
    );
    let tags: Vec<String> = testing::column(
        &lib.db,
        "SELECT t.name FROM work_tags wt JOIN tags t ON t.id = wt.tag_id WHERE wt.work_id = 3",
    )
    .await;
    assert_eq!(tags, ["中文标签"]);
}

#[tokio::test]
async fn a_work_neither_source_has_stays_without_metadata() {
    let lib = Lib::new().await;
    let (_, _, chain) = chain(&[]).await;
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 3, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::NotFound);
    assert_eq!(lib.work(3).await["metadata_src"], "none");
}

#[tokio::test]
async fn a_dlsite_outage_falls_back_and_a_later_refresh_returns_to_dlsite() {
    let lib = Lib::new().await;
    let (dlsite, _, chain) = chain(&[1000001]).await;
    dlsite
        .failures
        .lock()
        .unwrap()
        .insert("RJ01000001".into(), 10);
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    assert_eq!(lib.work(1).await["metadata_src"], "asmr_one");

    dlsite.failures.lock().unwrap().clear();
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 1, false, 200)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(1).await;
    assert_eq!(w["metadata_src"], "dlsite");
    assert_eq!(w["title"], "Title of RJ01000001");
    assert_eq!(
        w["cover_src"], "dlsite",
        "an asmr.one cover is not protected"
    );
}

#[tokio::test]
async fn a_dlsite_cover_that_fails_is_taken_from_asmr_one() {
    let lib = Lib::new().await;
    let (dlsite, _, chain) = chain(&[1000001]).await;
    dlsite
        .failures
        .lock()
        .unwrap()
        .insert("img RJ01000001.jpg".into(), 10);
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    let w = lib.work(1).await;
    assert_eq!(w["metadata_src"], "dlsite");
    assert_eq!(w["cover_src"], "asmr_one");
    assert_eq!(
        std::fs::read(lib.dirs.covers.join("1.jpg")).unwrap(),
        b"asmr jpeg"
    );
}

#[tokio::test]
async fn both_sources_down_leave_the_work_for_the_next_scan() {
    let lib = Lib::new().await;
    let (dlsite_fake, dlsite) = fake_dlsite(Duration::ZERO).await;
    let (_, down) = asmr_mirror(&[1000001], true).await;
    let chain = ProviderChain::new(Arc::new(dlsite), Some(Arc::new(asmr_provider(vec![down]))));
    dlsite_fake
        .failures
        .lock()
        .unwrap()
        .insert("RJ01000001".into(), 10);
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Unavailable(_)),
        "{outcome:?}"
    );
    assert_eq!(lib.work(1).await["metadata_src"], "none");
}

#[tokio::test]
async fn the_first_mirror_that_answers_is_used_and_kept() {
    let (down_fake, down) = asmr_mirror(&[], true).await;
    let (up_fake, up) = asmr_mirror(&[1000001, 1000002], false).await;
    let provider = asmr_provider(vec![down, up]);
    provider.fetch("RJ01000001").await.unwrap();
    provider.fetch("RJ01000002").await.unwrap();
    assert_eq!(*down_fake.log.lock().unwrap(), ["/api/workInfo/1"]);
    assert_eq!(
        *up_fake.log.lock().unwrap(),
        [
            "/api/workInfo/1",
            "/api/workInfo/1000001",
            "/api/workInfo/1000002"
        ],
        "the probe's 404 counts as an answer, and the choice is cached"
    );
    assert_eq!(
        provider.fetch("RJ99999999").await,
        Err(MetadataError::NotFound)
    );
}

#[tokio::test]
async fn an_asmr_one_cover_redirected_off_the_mirror_is_missing_not_down() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base: url::Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let app = Router::new().fallback(|| async {
        (
            StatusCode::FOUND,
            [(axum::http::header::LOCATION, "http://10.0.0.1/cover.jpg")],
        )
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = asmr_provider(vec![base.clone()]);
    let cover = base.join("api/cover/1.jpg").unwrap();
    assert_eq!(provider.cover(&cover).await, Err(MetadataError::NotFound));
}

#[tokio::test]
async fn asmr_one_covers_outside_the_mirrors_are_refused() {
    let (_, mirror) = asmr_mirror(&[], false).await;
    let provider = asmr_provider(vec![mirror]);
    let elsewhere: url::Url = "http://127.0.0.1:9/cover.jpg".parse().unwrap();
    assert_eq!(
        provider.cover(&elsewhere).await,
        Err(MetadataError::NotFound)
    );
}

/// How the fake hvdb answers.
#[derive(Clone, Copy, PartialEq)]
enum HvdbMode {
    /// Pages for the known numbers, 500 for the rest, as the real one does.
    Normal,
    /// 503 for everything.
    Down,
}

struct FakeHvdb {
    known: Vec<u64>,
    mode: HvdbMode,
    /// Requests with the time they arrived.
    log: Mutex<Vec<(String, Instant)>>,
}

/// A fake hvdb: `Details/{number}` is a work page with one voice actor in
/// romaji and in Japanese.
async fn hvdb_site(known: &[u64], mode: HvdbMode) -> (Arc<FakeHvdb>, url::Url) {
    let fake = Arc::new(FakeHvdb {
        known: known.to_vec(),
        mode,
        log: Mutex::new(Vec::new()),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let handle = {
        let fake = fake.clone();
        move |uri: axum::http::Uri| {
            let fake = fake.clone();
            async move {
                let path = uri.path().to_owned();
                fake.log
                    .lock()
                    .unwrap()
                    .push((path.clone(), Instant::now()));
                if fake.mode == HvdbMode::Down {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                let Some(n) = path
                    .strip_prefix("/Dashboard/Details/")
                    .and_then(|n| n.parse::<u64>().ok())
                    .filter(|n| fake.known.contains(n))
                else {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };
                axum::response::Html(format!(
                    r#"<html><head><title>HVDB - RJ{n:08}</title></head><body>
                    <a href="/Dashboard/TagWorks/5">ear cleaning</a>
                    <a href="/Dashboard/CVWorks/1">Koe Hitoe</a>
                    <a href="/Dashboard/CVWorks/2">&#x58F0;&#x4E00;&#x91CD;</a>
                    </body></html>"#
                ))
                .into_response()
            }
        }
    };
    let app = Router::new().fallback(handle);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, base.parse().unwrap())
}

/// DLsite, with hvdb for missing voice actors.
async fn chain_with_hvdb(known: &[u64], mode: HvdbMode) -> (Arc<FakeHvdb>, ProviderChain) {
    let (_, dlsite) = fake_dlsite(Duration::ZERO).await;
    let (fake, base) = hvdb_site(known, mode).await;
    let hvdb = HvdbProvider::new(HvdbConfig {
        base,
        spacing: Duration::ZERO,
        backoff: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..HvdbConfig::default()
    })
    .unwrap();
    let chain = ProviderChain::new(Arc::new(dlsite), None).with_voice_actors(Arc::new(hvdb));
    (fake, chain)
}

async fn voice_actors(lib: &Lib, work_id: i64) -> Vec<String> {
    testing::column(
        &lib.db,
        &format!(
            "SELECT v.name FROM work_vas wv JOIN vas v ON v.id = wv.va_id
             WHERE wv.work_id = {work_id} ORDER BY v.name"
        ),
    )
    .await
}

async fn add_work_without_voice_actors(lib: &Lib) {
    exec(&lib.db, "INSERT INTO works (id, code, title, metadata_src, cover_src, extra, created_at, updated_at)
         VALUES (5, 'RJ01000005', 'dir name', 'none', 'none', '{}', 0, 0)").await;
}

#[tokio::test]
async fn hvdb_fills_in_voice_actors_dlsite_does_not_list() {
    let lib = Lib::new().await;
    add_work_without_voice_actors(&lib).await;
    let (hvdb, chain) = chain_with_hvdb(&[1000005], HvdbMode::Normal).await;
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 5, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    assert_eq!(
        voice_actors(&lib, 5).await,
        ["声一重"],
        "the romaji twin is dropped"
    );
    let w = lib.work(5).await;
    assert_eq!(w["metadata_src"], "dlsite", "hvdb is not a metadata source");
    assert_eq!(w["title"], "Title of RJ01000005");
    let log = hvdb.log.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "/Dashboard/Details/1000005");
}

#[tokio::test]
async fn hvdb_is_not_asked_when_dlsite_lists_voice_actors() {
    let lib = Lib::new().await;
    let (hvdb, chain) = chain_with_hvdb(&[1000001], HvdbMode::Normal).await;
    fetch_and_store(&lib.db, &chain, &lib.dirs, 1, false, 100)
        .await
        .unwrap();
    assert_eq!(voice_actors(&lib, 1).await, ["Voice A", "Voice B"]);
    assert!(hvdb.log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_work_hvdb_cannot_answer_for_keeps_its_other_metadata() {
    // Unknown to hvdb: one 500, not retried.
    let lib = Lib::new().await;
    add_work_without_voice_actors(&lib).await;
    let (hvdb, chain) = chain_with_hvdb(&[], HvdbMode::Normal).await;
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 5, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    assert!(voice_actors(&lib, 5).await.is_empty());
    assert_eq!(lib.work(5).await["title"], "Title of RJ01000005");
    assert_eq!(hvdb.log.lock().unwrap().len(), 1, "a 500 is not retried");

    // hvdb is down: retried, then the work is stored all the same.
    let lib = Lib::new().await;
    add_work_without_voice_actors(&lib).await;
    let (hvdb, chain) = chain_with_hvdb(&[1000005], HvdbMode::Down).await;
    let outcome = fetch_and_store(&lib.db, &chain, &lib.dirs, 5, false, 100)
        .await
        .unwrap();
    assert_eq!(outcome, FetchOutcome::Updated);
    assert!(voice_actors(&lib, 5).await.is_empty());
    assert_eq!(lib.work(5).await["metadata_src"], "dlsite");
    assert_eq!(hvdb.log.lock().unwrap().len(), 4, "one try and 3 retries");
}

#[tokio::test(start_paused = true)]
async fn hvdb_requests_are_spaced_and_follow_no_redirect() {
    // An unsupported scheme fails before network I/O, so only the limiter
    // and zero-length retry backoffs advance Tokio's paused clock.
    let hvdb = HvdbProvider::new(HvdbConfig {
        base: "ftp://hvdb.invalid/".parse().unwrap(),
        spacing: Duration::from_millis(200),
        retries: 3,
        backoff: Duration::ZERO,
        ..HvdbConfig::default()
    })
    .unwrap();
    let start = Instant::now();
    // The first attempt is immediate. Later attempts, including the first
    // attempt of the next lookup, each wait another 200 ms.
    for elapsed_ms in [600, 1400] {
        assert!(matches!(
            hvdb.voice_actors("RJ01000005").await,
            Err(MetadataError::Unavailable(_))
        ));
        assert_eq!(start.elapsed(), Duration::from_millis(elapsed_ms));
    }

    // Real HTTP checks use the running clock so I/O cannot auto-advance
    // virtual time to a request timeout.
    tokio::time::resume();
    let (fake, base) = hvdb_site(&[1000005], HvdbMode::Down).await;
    let hvdb = HvdbProvider::new(HvdbConfig {
        base,
        spacing: Duration::ZERO,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        ..HvdbConfig::default()
    })
    .unwrap();
    // One lookup and its retries, then another lookup.
    for _ in 0..2 {
        assert!(matches!(
            hvdb.voice_actors("RJ01000005").await,
            Err(MetadataError::Unavailable(_))
        ));
    }
    assert_eq!(fake.log.lock().unwrap().len(), 8);
    assert!(hvdb.voice_actors("BJ01000005").await.unwrap().is_empty());
    assert_eq!(fake.log.lock().unwrap().len(), 8, "only RJ works are asked");

    // A redirect is not followed, even on hvdb's own host: the hop would
    // be a request the spacing never saw.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let moved = Arc::new(Mutex::new(0));
    let app = Router::new()
        .route(
            "/moved",
            get({
                let moved = moved.clone();
                move || async move {
                    *moved.lock().unwrap() += 1;
                    "<title>HVDB - RJ01000005</title>"
                }
            }),
        )
        .fallback(|| async { axum::response::Redirect::to("/moved") });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let hvdb = HvdbProvider::new(HvdbConfig {
        base: base.parse().unwrap(),
        spacing: Duration::ZERO,
        ..HvdbConfig::default()
    })
    .unwrap();
    assert_eq!(
        hvdb.voice_actors("RJ01000005").await,
        Err(MetadataError::NotFound)
    );
    assert_eq!(*moved.lock().unwrap(), 0);
}
