//! Partial listings sync readable siblings without trusting failed subtrees.

use super::{NOW, fake_openlist as openlist, fake_webdav as webdav, set_mode};
use neo_db::{Db, media, roots, scan, testing};
use neo_scanner::{NoObserver, Observer, Root, scan_root};
use neo_storage::{LocalStorage, OpenListStorage, Storage, WebDavStorage};
use std::{fs, sync::Mutex};

#[derive(Default)]
struct Probes(Mutex<Vec<i64>>);

impl Observer for Probes {
    fn probe(&self, ids: &[i64]) {
        self.0.lock().unwrap().extend_from_slice(ids);
    }
}

#[tokio::test]
async fn partial_walks_do_not_requeue_failed_probes_but_source_migration_does() {
    let server = webdav::start().await;
    for base in ["/media", "/moved"] {
        server.fake.dir(base);
        server.fake.dir(&format!("{base}/RJ123456"));
        server.fake.dir(&format!("{base}/RJ123456/blocked"));
        server.fake.dir(&format!("{base}/RJ123456/readable"));
        server.fake.file(
            &format!("{base}/RJ123456/readable/broken.mp3"),
            b"not audio",
        );
    }
    let test_db = testing::open().await;
    let db = &test_db.db;
    let mut stored = roots::StoredRoot {
        id: "test".into(),
        display_name: "Synthetic".into(),
        kind: "webdav".into(),
        settings: serde_json::json!({"url": server.url(), "base_path": "/media"}).to_string(),
    };
    assert!(roots::insert_stored(db, &stored, NOW).await.unwrap());
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    let mut root = Root {
        id: "test",
        storage: &storage,
        probe_after_scan: true,
        skip_unchanged: false,
    };
    let order = ["test".to_owned()];
    let probes = Probes::default();
    let first = scan_root(db, &root, &order, &probes, NOW).await.unwrap();
    assert_eq!((first.tracks_new, first.works_failed), (1, 0));
    assert_eq!(first.probe.len(), 1);
    assert_eq!(*probes.0.lock().unwrap(), first.probe);
    let track_id = first.probe[0];
    // A failed probe writes no result. Leave it empty while the file's
    // size and modification time stay unchanged through every scan.
    let before = media::track_file(db, track_id).await.unwrap().unwrap();
    assert_eq!(before.probe, None);
    probes.0.lock().unwrap().clear();

    for round in 1..=3 {
        server.fake.fault(
            "/media/RJ123456/blocked",
            webdav::Fault::Status {
                status: 403,
                times: 1,
                retry_after: None,
            },
        );
        let partial = scan_root(db, &root, &order, &probes, NOW + round)
            .await
            .unwrap();
        assert_eq!((partial.works_failed, partial.works_unchanged), (1, 0));
        assert_eq!((partial.tracks_new, partial.tracks_changed), (0, 0));
        assert!(partial.probe.is_empty(), "round {round}: {partial:?}");
        assert!(probes.0.lock().unwrap().is_empty(), "round {round}");
        // Later scans may compare the top level, but must still walk.
        root.skip_unchanged = true;
    }

    stored.settings = serde_json::json!({"url": server.url(), "base_path": "/moved"}).to_string();
    assert!(
        roots::update_stored(db, &stored, true, NOW + 4)
            .await
            .unwrap()
    );
    let moved_storage = WebDavStorage::new(server.client(), "/moved").unwrap();
    root.storage = &moved_storage;
    let migrated = scan_root(db, &root, &order, &probes, NOW + 4)
        .await
        .unwrap();
    assert_eq!((migrated.tracks_new, migrated.tracks_changed), (0, 0));
    assert_eq!((migrated.works_failed, migrated.works_unchanged), (0, 0));
    assert_eq!(migrated.probe, [track_id]);
    assert_eq!(*probes.0.lock().unwrap(), [track_id]);
    let after = media::track_file(db, track_id).await.unwrap().unwrap();
    assert_eq!((after.size, after.mtime), (before.size, before.mtime));
    assert_eq!(after.source_version, before.source_version + 1);

    probes.0.lock().unwrap().clear();
    root.skip_unchanged = false;
    let next = scan_root(db, &root, &order, &probes, NOW + 5)
        .await
        .unwrap();
    assert!(next.probe.is_empty());
    assert!(probes.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn source_migration_reprobes_a_subtree_after_a_partial_walk_recovers() {
    let server = webdav::start().await;
    for base in ["/media", "/moved"] {
        server.fake.dir(base);
        server.fake.dir(&format!("{base}/RJ123456"));
        server.fake.dir(&format!("{base}/RJ123456/blocked"));
        server.fake.dir(&format!("{base}/RJ123456/readable"));
        server.fake.file(
            &format!("{base}/RJ123456/blocked/track.mp3"),
            b"synthetic audio",
        );
        server.fake.file(
            &format!("{base}/RJ123456/readable/broken.mp3"),
            b"not audio",
        );
    }
    let test_db = testing::open().await;
    let db = &test_db.db;
    let mut stored = roots::StoredRoot {
        id: "test".into(),
        display_name: "Synthetic".into(),
        kind: "webdav".into(),
        settings: serde_json::json!({"url": server.url(), "base_path": "/media"}).to_string(),
    };
    assert!(roots::insert_stored(db, &stored, NOW).await.unwrap());
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    let mut root = Root {
        id: "test",
        storage: &storage,
        probe_after_scan: true,
        skip_unchanged: true,
    };
    let order = ["test".to_owned()];
    let probes = Probes::default();
    let first = scan_root(db, &root, &order, &probes, NOW).await.unwrap();
    assert_eq!((first.tracks_new, first.works_failed), (2, 0));
    assert_eq!(first.probe.len(), 2);
    let mut tracks = Vec::new();
    for id in first.probe {
        tracks.push(media::track_file(db, id).await.unwrap().unwrap());
    }
    let blocked = tracks
        .iter()
        .find(|t| t.display_path == "blocked/track.mp3")
        .unwrap();
    let readable = tracks
        .iter()
        .find(|t| t.display_path == "readable/broken.mp3")
        .unwrap();
    media::save_probe(db, blocked, "{}", Some(1_000))
        .await
        .unwrap();

    stored.settings = serde_json::json!({"url": server.url(), "base_path": "/moved"}).to_string();
    assert!(
        roots::update_stored(db, &stored, true, NOW + 1)
            .await
            .unwrap()
    );
    let invalidated = media::track_file(db, blocked.id).await.unwrap().unwrap();
    assert_eq!((invalidated.probe, invalidated.duration_ms), (None, None));
    assert_eq!(invalidated.source_version, blocked.source_version + 1);
    let moved_storage = WebDavStorage::new(server.client(), "/moved").unwrap();
    root.storage = &moved_storage;
    server.fake.fault(
        "/moved/RJ123456/blocked",
        webdav::Fault::Status {
            status: 403,
            times: 1,
            retry_after: None,
        },
    );
    probes.0.lock().unwrap().clear();
    let partial = scan_root(db, &root, &order, &probes, NOW + 1)
        .await
        .unwrap();
    assert_eq!((partial.works_failed, partial.works_unchanged), (1, 0));
    assert_eq!((partial.tracks_new, partial.tracks_changed), (0, 0));
    assert_eq!(partial.probe, [readable.id]);
    assert_eq!(*probes.0.lock().unwrap(), [readable.id]);

    // The readable file's probe fails again. Recovery must still queue the
    // blocked track, even though neither file's size or mtime has changed.
    probes.0.lock().unwrap().clear();
    let recovered = scan_root(db, &root, &order, &probes, NOW + 2)
        .await
        .unwrap();
    assert_eq!((recovered.works_failed, recovered.works_unchanged), (0, 0));
    assert_eq!((recovered.tracks_new, recovered.tracks_changed), (0, 0));
    assert!(recovered.probe.contains(&blocked.id), "{recovered:?}");
    assert!(recovered.probe.contains(&readable.id));
    assert_eq!(recovered.probe.len(), 2);
    assert_eq!(*probes.0.lock().unwrap(), recovered.probe);
    for before in &tracks {
        let after = media::track_file(db, before.id).await.unwrap().unwrap();
        assert_eq!((after.size, after.mtime), (before.size, before.mtime));
    }

    probes.0.lock().unwrap().clear();
    let unchanged = scan_root(db, &root, &order, &probes, NOW + 3)
        .await
        .unwrap();
    assert_eq!(unchanged.works_unchanged, 1);
    root.skip_unchanged = false;
    let full = scan_root(db, &root, &order, &probes, NOW + 4)
        .await
        .unwrap();
    assert!(full.probe.is_empty());
    assert!(probes.0.lock().unwrap().is_empty());
}

const INITIAL: &[(&str, &[u8])] = &[
    ("blocked/keep.mp3", b"one"),
    ("blocked/deep/remove.mp3", b"two"),
    ("blocked/restore.mp3", b"three"),
    ("blocked-sibling/remove.mp3", b"four"),
    ("blocked-sibling/change.mp3", b"five"),
    ("blocked-sibling/restore.mp3", b"six"),
];
const CHANGED: &[(&str, &[u8])] = &[
    ("blocked/keep.mp3", b"changed behind the failure"),
    ("blocked/restore.mp3", b"three"),
    ("blocked/new.mp3", b"new hidden track"),
    ("blocked-sibling/change.mp3", b"changed readable track"),
    ("blocked-sibling/restore.mp3", b"six"),
    ("blocked-sibling/new.mp3", b"new readable track"),
];

fn files(phase: u8) -> Vec<(&'static str, &'static [u8])> {
    if phase >= 2 {
        CHANGED.to_vec()
    } else {
        INITIAL
            .iter()
            .copied()
            .filter(|(path, _)| phase == 0 || !path.ends_with("restore.mp3"))
            .collect()
    }
}

#[tokio::test]
async fn local_partial_walk_syncs_readable_siblings_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("RJ123456");
    let blocked = work.join("blocked");
    fs::create_dir_all(&blocked).unwrap();
    set_mode(&blocked, 0o000);
    let unreadable = fs::read_dir(&blocked).is_err();
    set_mode(&blocked, 0o755);
    if !unreadable {
        eprintln!("skipping permission failure: this user can read mode 000 directories");
        return;
    }
    // Restore permissions even if an assertion fails, so TempDir can clean up.
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_mode(&self.0, 0o755);
        }
    }
    let _restore = Restore(blocked.clone());
    let storage = LocalStorage::new(dir.path());
    partial_walk_syncs_and_recovers(&storage, |phase| {
        set_mode(&blocked, 0o755);
        if phase < 3 {
            for (path, _) in INITIAL.iter().chain(CHANGED) {
                let path = work.join(path);
                if path.exists() {
                    fs::remove_file(path).unwrap();
                }
            }
            for (path, bytes) in files(phase) {
                let path = work.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, bytes).unwrap();
                super::set_mtime(&path, 1_800_000_000);
            }
        }
        if phase == 2 {
            set_mode(&blocked, 0o000);
        }
    })
    .await;
}

#[tokio::test]
async fn openlist_partial_walk_syncs_readable_siblings_and_recovers() {
    let server = openlist::start().await;
    let client = server.client();
    let storage = OpenListStorage::new(client.clone(), "/media").unwrap();
    partial_walk_syncs_and_recovers(&storage, |phase| {
        if phase < 3 {
            server.fake.put("/media", vec![openlist::dir("RJ123456")]);
            server.fake.put(
                "/media/RJ123456",
                vec![openlist::dir("blocked"), openlist::dir("blocked-sibling")],
            );
            for sub in ["blocked", "blocked/deep", "blocked-sibling"] {
                let mut entries: Vec<_> = files(phase)
                    .iter()
                    .filter_map(|(path, bytes)| {
                        let (parent, name) = path.rsplit_once('/').unwrap();
                        (parent == sub).then(|| openlist::file(name, bytes.len() as u64))
                    })
                    .collect();
                if sub == "blocked" {
                    entries.push(openlist::dir("deep"));
                }
                server.fake.put(&format!("/media/RJ123456/{sub}"), entries);
            }
        }
        for path in [
            "/media",
            "/media/RJ123456",
            "/media/RJ123456/blocked",
            "/media/RJ123456/blocked/deep",
            "/media/RJ123456/blocked-sibling",
        ] {
            client.invalidate_listing(path);
        }
        if phase == 2 {
            server.fake.fault(
                "list",
                "/media/RJ123456/blocked",
                openlist::Fault::Code(403, "denied"),
            );
        }
    })
    .await;
}

#[tokio::test]
async fn webdav_partial_walk_syncs_readable_siblings_and_recovers() {
    let server = webdav::start().await;
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    partial_walk_syncs_and_recovers(&storage, |phase| {
        if phase < 3 {
            server.fake.remove("/media");
            for path in [
                "/media",
                "/media/RJ123456",
                "/media/RJ123456/blocked",
                "/media/RJ123456/blocked/deep",
                "/media/RJ123456/blocked-sibling",
            ] {
                server.fake.dir(path);
            }
            for (path, bytes) in files(phase) {
                server.fake.file(&format!("/media/RJ123456/{path}"), bytes);
            }
        }
        if phase == 2 {
            server.fake.fault(
                "/media/RJ123456/blocked",
                webdav::Fault::Status {
                    status: 403,
                    times: 1,
                    retry_after: None,
                },
            );
        }
    })
    .await;
}

async fn partial_walk_syncs_and_recovers(storage: &dyn Storage, mut change: impl FnMut(u8)) {
    let test_db = testing::open().await;
    let db = &test_db.db;
    roots::sync(db, &[("test", "Synthetic")]).await.unwrap();
    let mut root = Root {
        id: "test",
        storage,
        probe_after_scan: false,
        skip_unchanged: false,
    };
    let order = ["test".to_owned()];
    change(0);
    let first = scan_root(db, &root, &order, &NoObserver, NOW)
        .await
        .unwrap();
    assert_eq!(first.tracks_new, 6);
    change(1);
    let second = scan_root(db, &root, &order, &NoObserver, NOW + 1)
        .await
        .unwrap();
    assert_eq!(second.tracks_missing, 2);
    // A former file can now be a directory. The failure protects the exact
    // path as well as its descendants, without protecting blocked-sibling.
    let mut tx = db.begin().await.unwrap();
    scan::insert_track(
        &mut tx,
        1,
        1,
        b"blocked",
        scan::ListedFile {
            display: "blocked",
            sort_key: "blocked",
            kind: "other",
            size: Some(7),
            mtime: None,
        },
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let before = protected_tracks(db).await;
    assert_eq!(before.len(), 4);
    assert_eq!(scan::stored_digests(db, "test").await.unwrap().len(), 1);
    testing::exec(db, r#"
        INSERT INTO users (name, is_admin, created_at) VALUES ('synthetic', FALSE, 0);
        INSERT INTO play_progress (user_id, work_id, position_ms, updated_at) VALUES (1, 1, 5, 0);
        INSERT INTO import_pending (user_id, work_id, source, state) VALUES
          (1, 1, '{"item":{"title":"new.mp3","subtitle":"blocked-sibling"},"seconds":1}', 'pending');
    "#).await;

    change(2);
    let partial = scan_root(db, &root, &order, &NoObserver, NOW + 2)
        .await
        .unwrap();
    assert_eq!(
        (partial.tracks_new, partial.tracks_missing),
        (1, 1),
        "{partial:?}"
    );
    assert_eq!(
        (partial.tracks_changed, partial.tracks_restored),
        (1, 1),
        "{partial:?}"
    );
    assert_eq!(protected_tracks(db).await, before);
    assert_eq!(partial.works_failed, 1);
    assert_eq!(partial.works_unchanged, 0);
    assert!(
        partial
            .log
            .iter()
            .any(|line| line.contains("RJ123456/blocked")
                && (line.contains("denied") || line.contains("403"))),
        "{:?}",
        partial.log
    );
    assert_eq!(
        scan::stored_digests(db, "test").await.unwrap(),
        [(1, String::new())]
    );
    let listed: Option<i64> = testing::value(db, "SELECT listed_at FROM work_locations").await;
    assert_eq!(listed, None);
    let state: String = testing::value(db, "SELECT state FROM import_pending").await;
    assert_eq!(state, "pending");
    assert_eq!(partial.pending_matched, 0);

    // A later scan of another root must not bypass the incomplete location.
    let other = tempfile::tempdir().unwrap();
    fs::create_dir(other.path().join("RJ123456")).unwrap();
    fs::write(other.path().join("RJ123456/note.txt"), b"note").unwrap();
    roots::sync(db, &[("other", "Other synthetic root")])
        .await
        .unwrap();
    let other_storage = LocalStorage::new(other.path());
    let deferred = scan_root(
        db,
        &Root {
            id: "other",
            storage: &other_storage,
            probe_after_scan: false,
            skip_unchanged: false,
        },
        &["test".to_owned(), "other".to_owned()],
        &NoObserver,
        NOW + 2,
    )
    .await
    .unwrap();
    assert_eq!(
        (deferred.pending_matched, deferred.pending_deferred),
        (0, 1)
    );

    change(3);
    root.skip_unchanged = true;
    let recovered = scan_root(db, &root, &order, &NoObserver, NOW + 3)
        .await
        .unwrap();
    assert_eq!((recovered.works_failed, recovered.works_unchanged), (0, 0));
    assert_eq!(
        (
            recovered.tracks_new,
            recovered.tracks_changed,
            recovered.tracks_missing,
            recovered.tracks_restored
        ),
        (1, 1, 2, 1),
        "{recovered:?}"
    );
    assert_eq!(recovered.pending_matched, 1);
    assert_eq!(scan::stored_digests(db, "test").await.unwrap().len(), 1);
    let after = protected_tracks(db).await;
    for track in before {
        assert_eq!(
            after.iter().find(|row| row.1 == track.1).unwrap().0,
            track.0
        );
    }
}

async fn protected_tracks(db: &Db) -> Vec<(i64, String, Option<i64>, Option<i64>, Option<i64>)> {
    testing::rows(db, "SELECT id, display_path, size, mtime, missing_at FROM tracks WHERE display_path = 'blocked' OR display_path LIKE 'blocked/%' ORDER BY id").await
}
