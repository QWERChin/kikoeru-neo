//! Empty remote roots preserve the library through the unavailable-root path.

#[path = "../../../neo-storage/tests/fake_openlist/mod.rs"]
mod openlist;

use super::fake_webdav as webdav;

use neo_db::{roots, scan, testing};
use neo_scanner::{NoObserver, Root, ScanError, scan_root};
use neo_storage::{OpenListStorage, Storage, StorageError, WebDavStorage};

#[tokio::test]
async fn an_empty_openlist_root_preserves_existing_data() {
    let server = openlist::start().await;
    server.fake.put(
        "/media",
        vec![openlist::dir("RJ123456"), openlist::dir("RJ654321")],
    );
    for code in ["RJ123456", "RJ654321"] {
        server
            .fake
            .put(&format!("/media/{code}"), vec![openlist::file("01.mp3", 5)]);
    }
    let client = server.client();
    let storage = OpenListStorage::new(client.clone(), "/media").unwrap();
    empty_root_preserves_existing_data(&storage, || {
        server.fake.put("/media", vec![]);
        client.invalidate_listing("/media");
    })
    .await;
}

#[tokio::test]
async fn an_empty_webdav_root_preserves_existing_data() {
    let server = webdav::start().await;
    server.fake.dir("/media");
    for code in ["RJ123456", "RJ654321"] {
        server.fake.dir(&format!("/media/{code}"));
        server.fake.file(&format!("/media/{code}/01.mp3"), b"audio");
    }
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    empty_root_preserves_existing_data(&storage, || {
        server.fake.remove("/media/RJ123456");
        server.fake.remove("/media/RJ654321");
    })
    .await;
}

async fn empty_root_preserves_existing_data(storage: &dyn Storage, empty_root: impl FnOnce()) {
    let test_db = testing::open().await;
    let db = &test_db.db;
    roots::sync(db, &[("remote", "Remote")]).await.unwrap();
    let root = Root {
        id: "remote",
        storage,
        probe_after_scan: false,
        skip_unchanged: true,
    };
    let order = ["remote".to_owned()];
    let before = scan_root(db, &root, &order, &NoObserver, super::NOW)
        .await
        .unwrap();
    assert_eq!((before.works_found, before.tracks_new), (2, 2));
    let locations = scan::root_locations(db, "remote").await.unwrap();
    let mut tracks = Vec::new();
    for location in &locations {
        tracks.push(scan::location_tracks(db, location.id).await.unwrap());
    }

    empty_root();
    let result = scan_root(db, &root, &order, &NoObserver, super::NOW + 1).await;
    assert_eq!(scan::root_locations(db, "remote").await.unwrap(), locations);
    for (location, before) in locations.iter().zip(tracks) {
        assert_eq!(
            scan::location_tracks(db, location.id).await.unwrap(),
            before
        );
        let mut tx = db.begin().await.unwrap();
        let work = scan::work_state(&mut tx, location.work_id).await.unwrap();
        assert!(!work.missing && work.present);
        tx.commit().await.unwrap();
    }
    let status = roots::status(db, "remote").await.unwrap().unwrap();
    assert_eq!(status.status, "unavailable");
    assert_eq!(status.last_scan_at, Some(super::NOW));
    assert_eq!(status.work_count, 2);
    assert!(
        matches!(
            result,
            Err(ScanError::Unavailable(StorageError::Unavailable(_)))
        ),
        "{result:?}"
    );
    assert!(result.unwrap_err().to_string().contains("/media is empty"));
}
