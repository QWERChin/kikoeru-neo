//! Failed WebDAV properties must not turn directories into missing data.

use super::fake_webdav as fake;

use neo_db::{roots, scan, testing};
use neo_scanner::{NoObserver, Root, ScanError, scan_root};
use neo_storage::{RawPath, Storage, StorageError, WebDavStorage};

#[tokio::test]
async fn partial_work_properties_preserve_existing_data() {
    partial_properties_preserve_existing_data("").await;
}

#[tokio::test]
async fn partial_subdirectory_properties_preserve_existing_data() {
    partial_properties_preserve_existing_data("RJ123456").await;
}

async fn partial_properties_preserve_existing_data(listed: &str) {
    let server = fake::start().await;
    server.fake.dir("/media");
    server.fake.dir("/media/RJ123456");
    server.fake.dir("/media/RJ123456/sub");
    server.fake.file("/media/RJ123456/sub/01.mp3", b"audio");
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    let test_db = testing::open().await;
    let db = &test_db.db;
    roots::sync(db, &[("dav", "WebDAV")]).await.unwrap();
    let root = Root {
        id: "dav",
        storage: &storage,
        probe_after_scan: false,
        skip_unchanged: false,
    };
    let order = ["dav".to_owned()];
    let before = scan_root(db, &root, &order, &NoObserver, super::NOW)
        .await
        .unwrap();
    assert_eq!((before.works_found, before.tracks_new), (1, 1));
    let locations = scan::root_locations(db, "dav").await.unwrap();
    assert_eq!(locations.len(), 1);
    assert!(!locations[0].missing);
    let tracks = scan::location_tracks(db, locations[0].id).await.unwrap();
    assert_eq!(tracks.len(), 1);
    assert!(!tracks[0].missing);

    let (parent, child) = if listed.is_empty() {
        ("/media", "RJ123456")
    } else {
        ("/media/RJ123456", "sub")
    };
    server.fake.fault(
        parent,
        fake::Fault::Body(format!(
            "<D:multistatus xmlns:D=\"DAV:\">\
             <D:response><D:href>/dav{parent}/</D:href>\
               <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>\
               <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\
             <D:response><D:href>/dav{parent}/{child}/</D:href>\
               <D:propstat><D:prop><D:resourcetype/></D:prop>\
               <D:status>HTTP/1.1 500 Internal Server Error</D:status></D:propstat>\
               <D:propstat><D:prop><D:getetag>etag</D:getetag></D:prop>\
               <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\
             </D:multistatus>"
        )),
    );
    let result = scan_root(db, &root, &order, &NoObserver, super::NOW + 1).await;
    assert_eq!(scan::root_locations(db, "dav").await.unwrap(), locations);
    assert_eq!(
        scan::location_tracks(db, locations[0].id).await.unwrap(),
        tracks
    );
    let mut tx = db.begin().await.unwrap();
    let work = scan::work_state(&mut tx, locations[0].work_id)
        .await
        .unwrap();
    assert!(!work.missing && work.present);
    tx.commit().await.unwrap();

    let path = RawPath::new(listed.as_bytes().to_vec()).unwrap();
    assert!(matches!(
        storage.list(&path).await,
        Err(StorageError::Unavailable(_))
    ));
    if listed.is_empty() {
        assert!(matches!(
            result,
            Err(ScanError::Unavailable(StorageError::Unavailable(_)))
        ));
    } else {
        let summary = result.unwrap();
        assert_eq!(summary.works_failed, 1);
        assert_eq!(summary.tracks_missing, 0);
    }
}
