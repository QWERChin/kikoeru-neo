//! Scans through local and remote storage, checked against a temporary
//! database. Every name and file is synthetic.

#[path = "../../webdav-client/tests/common/mod.rs"]
mod fake_webdav;
#[path = "remote_scan/mod.rs"]
mod remote_scan;
#[path = "webdav_scan/mod.rs"]
mod webdav_scan;

use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

use neo_db::{
    Db,
    testing::{self, TestDb, exec},
};
use std::sync::Mutex;

use neo_scanner::{NoObserver, Observer, Root, ScanError, Summary, scan_root};
use neo_storage::{LocalStorage, StorageError};

const NOW: i64 = 1_800_000_000_000;

/// user, import_pending.state, track_id, position_ms, group_path
type PendingState = (i64, String, Option<i64>, i64, Option<String>);

struct Lib {
    _dir: tempfile::TempDir,
    _db: TestDb,
    base: PathBuf,
    db: Db,
}

impl Lib {
    async fn new(roots: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_owned();
        let t = testing::open().await;
        let db = t.db.clone();
        for id in roots {
            fs::create_dir_all(base.join(id)).unwrap();
            exec(
                &db,
                &format!("INSERT INTO roots (id, display_name) VALUES ('{id}', '{id}')"),
            )
            .await;
        }
        Self {
            _dir: dir,
            _db: t,
            base,
            db,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    async fn scan(&self, root: &str, order: &[&str]) -> Result<Summary, ScanError> {
        let storage = LocalStorage::new(self.path(root));
        let order: Vec<String> = order.iter().map(|s| (*s).to_owned()).collect();
        scan_root(
            &self.db,
            &Root {
                id: root,
                storage: &storage,
                probe_after_scan: true,
                skip_unchanged: false,
            },
            &order,
            &NoObserver,
            NOW,
        )
        .await
    }

    async fn ok(&self, root: &str) -> Summary {
        self.scan(root, &[root]).await.unwrap()
    }

    async fn i64(&self, sql: &str) -> i64 {
        testing::value(&self.db, sql).await
    }

    async fn track(&self, display: &str) -> (i64, Option<i64>, Option<i64>) {
        let display = display.replace('\'', "''");
        let rows: Vec<(i64, Option<i64>, Option<i64>)> = testing::rows(
            &self.db,
            &format!(
                "SELECT id, missing_at, duplicate_of FROM tracks
                 WHERE display_path = '{display}' ORDER BY id"
            ),
        )
        .await;
        rows.into_iter().next().expect("no such track")
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn discovers_works_and_keeps_track_ids_across_scans() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/group/RJ01000001 title/mp3/02.mp3", b"two");
    lib.write("a/group/RJ01000001 title/mp3/10.mp3", b"ten");
    lib.write("a/group/RJ01000001 title/cover.jpg", b"jpg");
    lib.write("a/RJ123456/readme.txt", b"txt");
    fs::write(
        lib.path("a/RJ123456")
            .join(OsStr::from_bytes(b"\xb2\xe2\xca\xd4.wav")),
        b"gbk",
    )
    .unwrap();
    lib.write("a/no code here/x.mp3", b"x");
    std::os::unix::fs::symlink(lib.path("a/RJ123456"), lib.path("a/RJ999999")).unwrap();

    let s = lib.ok("a").await;
    assert_eq!((s.works_found, s.works_new, s.tracks_new), (2, 2, 5));
    assert_eq!(s.probe.len(), 3, "audio only: {s:?}");
    assert!(
        s.log
            .iter()
            .any(|l| l.contains("skipped symlink: RJ999999")),
        "{:?}",
        s.log
    );

    let works: Vec<(String, String, String)> = testing::rows(
        &lib.db,
        "SELECT code, title, metadata_src FROM works ORDER BY code",
    )
    .await;
    assert_eq!(
        works,
        [
            (
                "RJ01000001".into(),
                "RJ01000001 title".into(),
                "none".into()
            ),
            ("RJ123456".into(), "RJ123456".into(), "none".into())
        ]
    );
    let tracks: Vec<(Vec<u8>, String, String, Option<i64>)> = testing::rows(
        &lib.db,
        "SELECT t.rel_path, t.display_path, t.kind, t.size FROM tracks t
         JOIN works w ON w.id = t.work_id ORDER BY w.code, t.sort_key",
    )
    .await;
    let shown: Vec<(&str, &str)> = tracks
        .iter()
        .map(|t| (t.1.as_str(), t.2.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("cover.jpg", "image"),
            ("mp3/02.mp3", "audio"),
            ("mp3/10.mp3", "audio"),
            ("readme.txt", "text"),
            ("测试.wav", "audio"),
        ]
    );
    assert_eq!(tracks[4].0, b"\xb2\xe2\xca\xd4.wav", "raw bytes are kept");
    assert_eq!(tracks[1].3, Some(3));

    let before = lib.track("mp3/10.mp3").await;
    let s = lib.ok("a").await;
    assert_eq!(
        (
            s.works_new,
            s.tracks_new,
            s.tracks_changed,
            s.tracks_missing
        ),
        (0, 0, 0, 0)
    );
    assert!(s.probe.is_empty());
    assert_eq!(lib.track("mp3/10.mp3").await, before);
    assert_eq!(lib.i64("SELECT last_scan_at FROM roots").await, NOW);
}

#[tokio::test]
async fn marks_missing_files_and_works_and_restores_them() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ123456/01.mp3", b"one");
    lib.write("a/RJ123456/02.mp3", b"two");
    lib.write("a/RJ654321/01.mp3", b"one");
    lib.ok("a").await;
    let (id, _, _) = lib.track("02.mp3").await;
    exec(&lib.db, "UPDATE tracks SET probe = '{}', duration_ms = 5").await;

    fs::remove_file(lib.path("a/RJ123456/02.mp3")).unwrap();
    fs::rename(lib.path("a/RJ654321"), lib.path("RJ654321-away")).unwrap();
    let s = lib.ok("a").await;
    assert_eq!(
        (s.tracks_missing, s.locations_missing, s.works_missing),
        (1, 1, 1),
        "{s:?}"
    );
    assert_eq!(lib.track("02.mp3").await, (id, Some(NOW), None));
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE missing_at IS NOT NULL")
            .await,
        2,
        "the missing work's track is missing too"
    );
    assert_eq!(
        lib.i64("SELECT missing_at FROM works WHERE code = 'RJ654321'")
            .await,
        NOW
    );

    // Back again, with the second file changed: same IDs, stale probe gone.
    lib.write("a/RJ123456/02.mp3", b"two, longer");
    fs::rename(lib.path("RJ654321-away"), lib.path("a/RJ654321")).unwrap();
    let s = lib.ok("a").await;
    assert_eq!(
        (s.tracks_changed, s.locations_restored, s.works_restored),
        (1, 1, 1)
    );
    assert_eq!(s.tracks_restored, 1);
    assert_eq!(s.probe, [id]);
    assert_eq!(lib.track("02.mp3").await, (id, None, None));
    assert_eq!(
        lib.i64(
            "SELECT COUNT(*) FROM tracks WHERE probe IS NULL AND duration_ms IS NULL AND id = 2"
        )
        .await,
        1
    );
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM works WHERE missing_at IS NOT NULL")
            .await,
        0
    );
}

#[tokio::test]
async fn an_unavailable_root_or_work_changes_nothing() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ123456/01.mp3", b"one");
    lib.write("a/RJ654321/01.mp3", b"one");
    lib.ok("a").await;

    // An unreadable work directory keeps its tracks as they are.
    set_mode(&lib.path("a/RJ654321"), 0o000);
    if fs::read_dir(lib.path("a/RJ654321")).is_err() {
        let s = lib.ok("a").await;
        set_mode(&lib.path("a/RJ654321"), 0o755);
        assert_eq!((s.works_failed, s.tracks_missing), (1, 0), "{s:?}");
    } else {
        set_mode(&lib.path("a/RJ654321"), 0o755);
    }

    // An unmounted disk leaves an empty mount point: not an empty library.
    fs::rename(lib.path("a"), lib.path("a-away")).unwrap();
    fs::create_dir(lib.path("a")).unwrap();
    assert!(matches!(
        lib.scan("a", &["a"]).await,
        Err(ScanError::Unavailable(_))
    ));
    fs::remove_dir(lib.path("a")).unwrap();

    // A root directory that is not there at all.
    fs::rename(lib.path("a-away"), lib.path("a-unmounted")).unwrap();
    let err = lib.scan("a", &["a"]).await.unwrap_err();
    assert!(
        matches!(err, ScanError::Unavailable(StorageError::Unavailable(_))),
        "{err:?}"
    );
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM works WHERE missing_at IS NOT NULL")
            .await,
        0
    );
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE missing_at IS NOT NULL")
            .await,
        0
    );
    let status: String = testing::value(&lib.db, "SELECT status FROM roots").await;
    assert_eq!(status, "unavailable");

    assert!(matches!(
        lib.scan("nope", &[]).await,
        Err(ScanError::UnknownRoot(_))
    ));
}

#[tokio::test]
async fn merges_locations_and_moves_progress_to_the_kept_copy() {
    let lib = Lib::new(&["a", "b"]).await;
    lib.write("a/RJ123456/mp3/01.mp3", b"same");
    lib.write("a/RJ123456/mp3/02.mp3", b"short");
    lib.write("b/x/RJ123456 wav/mp3/01.mp3", b"same");
    lib.write("b/x/RJ123456 wav/mp3/02.mp3", b"different size");
    lib.write("b/x/RJ123456 wav/wav/01.wav", b"wav");
    let order = ["b", "a"];
    lib.scan("a", &order).await.unwrap();
    let s = lib.scan("b", &order).await.unwrap();
    assert_eq!((s.works_new, s.locations_new), (0, 1));
    assert!(
        s.log
            .iter()
            .any(|l| l.contains("another location of RJ123456"))
    );
    assert_eq!(lib.i64("SELECT COUNT(*) FROM works").await, 1);

    // Same path and size: the copy in the preferred root b is kept.
    let ids: Vec<(i64, String, Option<i64>)> = testing::rows(
        &lib.db,
        "SELECT t.id, l.root_id, t.duplicate_of FROM tracks t
         JOIN work_locations l ON l.id = t.location_id
         WHERE t.display_path = 'mp3/01.mp3' ORDER BY l.root_id",
    )
    .await;
    let (a_copy, b_copy) = (ids[0].0, ids[1].0);
    assert_eq!(ids[0].2, Some(b_copy));
    assert_eq!(ids[1].2, None);
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE duplicate_of IS NOT NULL")
            .await,
        1,
        "02.mp3 differs in size, so both copies show"
    );

    // Progress on the kept copy moves when its location disappears.
    exec(
        &lib.db,
        &format!(
            "INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
             INSERT INTO play_progress (user_id, work_id, track_id, updated_at)
               VALUES (1, 1, {b_copy}, 0)"
        ),
    )
    .await;
    // Root b keeps another file, because an empty root reads as unmounted.
    lib.write("b/readme.txt", b"still mounted");
    fs::remove_dir_all(lib.path("b/x")).unwrap();
    lib.scan("b", &order).await.unwrap();
    // Re-scanning a promotes its copy.
    lib.scan("a", &order).await.unwrap();
    assert_eq!(lib.i64("SELECT track_id FROM play_progress").await, a_copy);
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE id = (SELECT track_id FROM play_progress) AND duplicate_of IS NULL").await,
        1
    );
}

/// An imported work at `RJ0123456` (the code is RJ123456) with a second
/// location, and four users' old positions: a unique path, a path two
/// files share, a path that does not exist yet, and one whose progress the
/// user already cleared in the new server.
#[tokio::test]
async fn matches_imported_progress_after_the_scan() {
    let lib = Lib::new(&["a"]).await;
    exec(
        &lib.db,
        r#"INSERT INTO works (code, title, metadata_src, extra, created_at, updated_at)
             VALUES ('RJ123456', 'old', 'dlsite',
                     '{"legacy_durations":{"mp3/SE/01.mp3":61.5,"02.mp3":90}}', 0, 0);
           INSERT INTO users (name, is_admin, created_at)
             VALUES ('u1', FALSE, 0), ('u2', FALSE, 0), ('u3', FALSE, 0), ('u4', FALSE, 0);
           INSERT INTO play_progress (user_id, work_id, position_ms, updated_at)
             VALUES (1, 1, 12500, 0), (2, 1, 1000, 0), (3, 1, 2000, 0);
           INSERT INTO import_pending (user_id, work_id, source, state) VALUES
             (1, 1, '{"item":{"title":"01.mp3","subtitle":"mp3/SE"},"seconds":12.5}', 'pending'),
             (2, 1, '{"item":{"title":"02.mp3","subtitle":""},"seconds":1}', 'pending'),
             (3, 1, '{"item":{"title":"03.mp3","subtitle":"mp3/SE"},"seconds":2}', 'pending'),
             (4, 1, '{"item":{"title":"01.mp3","subtitle":"mp3/SE"},"seconds":3}', 'pending');"#,
    )
    .await;
    lib.write("a/RJ0123456/mp3/SE/01.mp3", b"one");
    lib.write("a/RJ0123456/mp3/NoSE/01.mp3", b"one, no SE");
    lib.write("a/RJ0123456/02.mp3", b"two");
    lib.write("a/other/RJ123456/02.mp3", b"two, other size");
    let s = lib.ok("a").await;

    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM works").await,
        1,
        "RJ0123456 is RJ123456"
    );
    let (se, _, _) = lib.track("mp3/SE/01.mp3").await;
    assert_eq!(
        lib.i64("SELECT duration_ms FROM tracks WHERE display_path = 'mp3/SE/01.mp3'")
            .await,
        61_500,
        "the imported duration is the first hint"
    );
    let states: Vec<PendingState> = testing::rows(
        &lib.db,
        "SELECT p.user_id, p.state, g.track_id, g.position_ms, g.group_path
         FROM import_pending p JOIN play_progress g USING (user_id, work_id) ORDER BY p.user_id",
    )
    .await;
    assert_eq!(
        states[0],
        (1, "matched".into(), Some(se), 12_500, Some("mp3/SE".into()))
    );
    assert_eq!(
        states[1].1, "ambiguous",
        "02.mp3 exists twice with different sizes"
    );
    assert_eq!(states[1].2, None);
    assert_eq!(states[2].1, "pending", "03.mp3 does not exist yet");
    let cleared: String = testing::value(
        &lib.db,
        "SELECT state FROM import_pending WHERE user_id = 4",
    )
    .await;
    assert_eq!(cleared, "superseded");
    assert_eq!(
        (
            s.pending_matched,
            s.pending_ambiguous,
            s.pending_not_found,
            s.pending_superseded
        ),
        (1, 1, 1, 1)
    );

    // A later scan, after a restart in practice, matches what appeared since.
    lib.write("a/RJ0123456/mp3/SE/03.mp3", b"three");
    let s = lib.ok("a").await;
    assert_eq!(s.pending_matched, 1);
    let (three, _, _) = lib.track("mp3/SE/03.mp3").await;
    assert_eq!(
        lib.i64("SELECT track_id FROM play_progress WHERE user_id = 3")
            .await,
        three
    );
}

#[tokio::test]
async fn a_copy_in_another_root_takes_over_when_the_kept_one_goes() {
    let lib = Lib::new(&["a", "b"]).await;
    lib.write("a/RJ123456/01.mp3", b"same");
    lib.write("a/other/RJ654321/01.mp3", b"keeps root a mounted");
    lib.write("b/RJ123456/01.mp3", b"same");
    let order = ["a", "b"];
    lib.scan("a", &order).await.unwrap();
    lib.scan("b", &order).await.unwrap();
    let kept: i64 = lib
        .i64("SELECT id FROM tracks WHERE display_path = '01.mp3' AND duplicate_of IS NULL AND work_id = 1")
        .await;
    exec(
        &lib.db,
        &format!(
            "INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
             INSERT INTO play_progress (user_id, work_id, track_id, updated_at)
               VALUES (1, 1, {kept}, 0)"
        ),
    )
    .await;

    // Only root a is scanned after its copy disappears.
    fs::remove_dir_all(lib.path("a/RJ123456")).unwrap();
    lib.scan("a", &order).await.unwrap();
    let progress = lib.i64("SELECT track_id FROM play_progress").await;
    assert_ne!(progress, kept);
    assert_eq!(
        lib.i64(&format!(
            "SELECT COUNT(*) FROM tracks t JOIN work_locations l ON l.id = t.location_id
             WHERE t.id = {progress} AND l.root_id = 'b' AND t.duplicate_of IS NULL"
        ))
        .await,
        1
    );
}

/// Cancels the scan as soon as the first work's probes are handed over.
#[derive(Default)]
struct CancelAfterFirstProbe {
    probed: Mutex<Vec<i64>>,
}

impl Observer for CancelAfterFirstProbe {
    fn probe(&self, ids: &[i64]) {
        self.probed.lock().unwrap().extend_from_slice(ids);
    }

    fn is_cancelled(&self) -> bool {
        !self.probed.lock().unwrap().is_empty()
    }
}

#[tokio::test]
async fn a_cancelled_scan_hands_over_the_probes_it_found() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ123456/01.mp3", b"one");
    lib.write("a/RJ654321/01.mp3", b"two");
    let storage = LocalStorage::new(lib.path("a"));
    let observer = CancelAfterFirstProbe::default();
    let result = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &observer,
        NOW,
    )
    .await;
    assert!(matches!(result, Err(ScanError::Cancelled)));
    let probed = observer.probed.lock().unwrap().clone();
    assert_eq!(probed.len(), 1);
    assert_eq!(
        lib.i64(&format!(
            "SELECT COUNT(*) FROM tracks WHERE id = {}",
            probed[0]
        ))
        .await,
        1
    );
}

#[tokio::test]
async fn an_unlisted_location_defers_matching() {
    let lib = Lib::new(&["a"]).await;
    exec(
        &lib.db,
        r#"INSERT INTO works (code, title, metadata_src, created_at, updated_at)
             VALUES ('RJ123456', 'old', 'dlsite', 0, 0);
           INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
           INSERT INTO play_progress (user_id, work_id, position_ms, updated_at) VALUES (1, 1, 5, 0);
           INSERT INTO import_pending (user_id, work_id, source, state) VALUES
             (1, 1, '{"item":{"title":"01.mp3","subtitle":""},"seconds":1}', 'pending');"#,
    ).await;
    lib.write("a/RJ123456/01.mp3", b"one");
    lib.write("a/x/RJ123456/01.mp3", b"one, other version");
    set_mode(&lib.path("a/x/RJ123456"), 0o000);
    if fs::read_dir(lib.path("a/x/RJ123456")).is_ok() {
        // Running as root: permissions cannot hide the directory.
        return;
    }
    let s = lib.ok("a").await;
    set_mode(&lib.path("a/x/RJ123456"), 0o755);
    assert_eq!(s.works_failed, 1);
    assert_eq!(s.pending_matched, 0, "{s:?}");

    // Once both copies are listed, the old path turns out to be ambiguous.
    let s = lib.ok("a").await;
    assert_eq!(s.pending_ambiguous, 1, "{s:?}");
}

#[tokio::test]
async fn a_cancelled_scan_still_settles_duplicates() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ123456/01.mp3", b"same");
    lib.write("a/z/RJ123456/01.mp3", b"same");
    lib.write("a/zz/RJ654321/01.mp3", b"another work");
    lib.ok("a").await;
    // Complete the initial probes so cancellation follows the changed copy.
    exec(&lib.db, "UPDATE tracks SET probe = '{}'").await;
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE duplicate_of IS NOT NULL")
            .await,
        1
    );

    // The hidden copy changes. The scan is cancelled right after it is
    // synced, before the last work.
    lib.write("a/z/RJ123456/01.mp3", b"now a different file");
    let storage = LocalStorage::new(lib.path("a"));
    let observer = CancelAfterFirstProbe::default();
    let result = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &observer,
        NOW + 1,
    )
    .await;
    assert!(matches!(result, Err(ScanError::Cancelled)));
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE duplicate_of IS NOT NULL")
            .await,
        0,
        "the changed copy is a different file and must show"
    );
}

#[tokio::test]
async fn matching_waits_for_locations_in_roots_not_scanned_yet() {
    let lib = Lib::new(&["a", "b"]).await;
    // As the import leaves it: a location in root b that no scan has listed.
    exec(
        &lib.db,
        r#"INSERT INTO works (code, title, metadata_src, created_at, updated_at)
             VALUES ('RJ123456', 'old', 'dlsite', 0, 0);
           INSERT INTO work_locations (work_id, root_id, rel_path, display_path)
             VALUES (1, 'b', CAST('RJ123456' AS BLOB), 'RJ123456');
           INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
           INSERT INTO play_progress (user_id, work_id, position_ms, updated_at) VALUES (1, 1, 5, 0);
           INSERT INTO import_pending (user_id, work_id, source, state) VALUES
             (1, 1, '{"item":{"title":"02.mp3","subtitle":""},"seconds":1}', 'pending');"#,
    ).await;
    lib.write("a/RJ123456/02.mp3", b"two");
    lib.write("b/RJ123456/02.mp3", b"two, another recording");
    let order = ["a", "b"];
    let s = lib.scan("a", &order).await.unwrap();
    assert_eq!((s.pending_matched, s.pending_deferred), (0, 1), "{s:?}");
    let s = lib.scan("b", &order).await.unwrap();
    assert_eq!(s.pending_ambiguous, 1, "{s:?}");
}

#[tokio::test]
async fn a_cancelled_scan_promotes_copies_of_lost_locations() {
    let lib = Lib::new(&["a", "b"]).await;
    lib.write("a/RJ123456/01.mp3", b"same");
    lib.write("a/RJ654321/01.mp3", b"first work in a");
    lib.write("a/z/RJ999999/01.mp3", b"last work in a");
    lib.write("b/RJ123456/01.mp3", b"same");
    let order = ["a", "b"];
    lib.scan("a", &order).await.unwrap();
    lib.scan("b", &order).await.unwrap();
    let b_copy = lib
        .i64(
            "SELECT t.id FROM tracks t JOIN work_locations l ON l.id = t.location_id
             WHERE l.root_id = 'b'",
        )
        .await;
    assert!(
        testing::value::<bool>(
            &lib.db,
            &format!("SELECT duplicate_of IS NOT NULL FROM tracks WHERE id = {b_copy}"),
        )
        .await
    );

    // a's copy disappears, a file in a's first work changes so the scan
    // hands over a probe and is cancelled before the last work.
    fs::remove_dir_all(lib.path("a/RJ123456")).unwrap();
    lib.write("a/RJ654321/01.mp3", b"first work in a, changed");
    let storage = LocalStorage::new(lib.path("a"));
    let observer = CancelAfterFirstProbe::default();
    let result = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &order.map(str::to_owned),
        &observer,
        NOW + 1,
    )
    .await;
    assert!(matches!(result, Err(ScanError::Cancelled)));
    assert!(
        testing::value::<bool>(
            &lib.db,
            &format!("SELECT duplicate_of IS NULL FROM tracks WHERE id = {b_copy}"),
        )
        .await,
        "b's copy takes over"
    );
}

#[tokio::test]
async fn a_cancel_during_the_last_work_skips_matching() {
    let lib = Lib::new(&["a"]).await;
    exec(
        &lib.db,
        r#"INSERT INTO works (code, title, metadata_src, created_at, updated_at)
             VALUES ('RJ123456', 'old', 'dlsite', 0, 0);
           INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
           INSERT INTO play_progress (user_id, work_id, position_ms, updated_at) VALUES (1, 1, 5, 0);
           INSERT INTO import_pending (user_id, work_id, source, state) VALUES
             (1, 1, '{"item":{"title":"01.mp3","subtitle":""},"seconds":1}', 'pending');"#,
    ).await;
    lib.write("a/RJ123456/01.mp3", b"one");
    let storage = LocalStorage::new(lib.path("a"));
    let observer = CancelAfterFirstProbe::default();
    let result = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &observer,
        NOW,
    )
    .await;
    assert!(matches!(result, Err(ScanError::Cancelled)));
    let state: String = testing::value(&lib.db, "SELECT state FROM import_pending").await;
    assert_eq!(state, "pending");
}

#[tokio::test]
async fn progress_reported_on_a_hidden_copy_moves_to_the_kept_one() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ123456/01.mp3", b"same");
    lib.write("a/x/RJ123456/01.mp3", b"same");
    lib.ok("a").await;
    let hidden = lib
        .i64("SELECT id FROM tracks WHERE duplicate_of IS NOT NULL")
        .await;
    let kept = lib.i64("SELECT duplicate_of FROM tracks WHERE id = (SELECT id FROM tracks WHERE duplicate_of IS NOT NULL)").await;
    // A player that still had the hidden copy's ID reports progress on it.
    exec(
        &lib.db,
        &format!(
            "INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
             INSERT INTO play_progress (user_id, work_id, track_id, updated_at)
               VALUES (1, 1, {hidden}, 0)"
        ),
    )
    .await;
    lib.ok("a").await;
    assert_eq!(lib.i64("SELECT track_id FROM play_progress").await, kept);
}

/// Cancels once every location has been synced, while the scan settles
/// duplicates and matches imports.
#[derive(Default)]
struct CancelAfterTracks {
    cancelled: Mutex<bool>,
}

impl Observer for CancelAfterTracks {
    fn progress(&self, stage: neo_scanner::Stage, done: u64, total: u64) {
        if stage == neo_scanner::Stage::Tracks && done == total {
            *self.cancelled.lock().unwrap() = true;
        }
    }

    fn is_cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap()
    }
}

#[tokio::test]
async fn a_cancel_after_syncing_stops_matching() {
    let lib = Lib::new(&["a"]).await;
    exec(
        &lib.db,
        r#"INSERT INTO works (code, title, metadata_src, created_at, updated_at)
             VALUES ('RJ123456', 'one', 'dlsite', 0, 0), ('RJ654321', 'two', 'dlsite', 0, 0);
           INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
           INSERT INTO play_progress (user_id, work_id, position_ms, updated_at)
             VALUES (1, 1, 5, 0), (1, 2, 5, 0);
           INSERT INTO import_pending (user_id, work_id, source, state) VALUES
             (1, 1, '{"item":{"title":"01.mp3","subtitle":""},"seconds":1}', 'pending'),
             (1, 2, '{"item":{"title":"01.mp3","subtitle":""},"seconds":1}', 'pending');"#,
    )
    .await;
    lib.write("a/RJ123456/01.mp3", b"one");
    lib.write("a/RJ654321/01.mp3", b"two");
    let storage = LocalStorage::new(lib.path("a"));
    let result = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: false,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &CancelAfterTracks::default(),
        NOW,
    )
    .await;
    assert!(matches!(result, Err(ScanError::Cancelled)));
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM import_pending WHERE state = 'pending'")
            .await,
        2
    );
}

impl Lib {
    /// A scan as of an OpenList root: works whose top level is unchanged
    /// are not walked.
    async fn skipping(&self, root: &str) -> Summary {
        let storage = LocalStorage::new(self.path(root));
        scan_root(
            &self.db,
            &Root {
                id: root,
                storage: &storage,
                probe_after_scan: false,
                skip_unchanged: true,
            },
            &[root.to_owned()],
            &NoObserver,
            NOW,
        )
        .await
        .unwrap()
    }
}

fn set_mtime(path: &Path, secs: u64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[tokio::test]
async fn a_work_with_an_unchanged_top_level_is_not_walked() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ000001/mp3/01.mp3", b"one");
    lib.write("a/RJ000001/cover.jpg", b"jpg");

    let s = lib.skipping("a").await;
    assert_eq!((s.tracks_new, s.works_unchanged), (2, 0), "no digest yet");

    // Same size, new time, one level down: the top level does not change.
    lib.write("a/RJ000001/mp3/01.mp3", b"uno");
    set_mtime(&lib.path("a/RJ000001/mp3/01.mp3"), 1_900_000_000);
    let s = lib.skipping("a").await;
    assert_eq!((s.tracks_changed, s.works_unchanged), (0, 1));

    // A full scan walks it anyway.
    let s = lib.ok("a").await;
    assert_eq!((s.tracks_changed, s.works_unchanged), (1, 0));

    lib.write("a/RJ000001/readme.txt", b"txt");
    let s = lib.skipping("a").await;
    assert_eq!((s.tracks_new, s.works_unchanged), (1, 0));
}

#[tokio::test]
async fn a_location_found_again_is_walked() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ000001/01.mp3", b"one");
    // An empty root is refused as an unmounted disk.
    lib.write("a/RJ000002/01.mp3", b"two");
    lib.skipping("a").await;

    fs::rename(lib.path("a/RJ000001"), lib.path("away")).unwrap();
    let s = lib.skipping("a").await;
    assert_eq!((s.locations_missing, s.tracks_missing), (1, 0));
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM tracks WHERE missing_at IS NOT NULL")
            .await,
        1
    );

    // The top level is the same as before it went, but its tracks were
    // marked missing with it and must be restored.
    fs::rename(lib.path("away"), lib.path("a/RJ000001")).unwrap();
    let s = lib.skipping("a").await;
    assert_eq!(
        (s.tracks_restored, s.works_unchanged),
        (1, 1),
        "only RJ000002 is skipped"
    );
}

/// A local root whose listings repeat the first file of every directory,
/// as an OpenList listing can.
struct Repeating(LocalStorage);

#[async_trait::async_trait]
impl neo_storage::Storage for Repeating {
    async fn list(
        &self,
        dir: &neo_storage::RawPath,
    ) -> Result<Vec<neo_storage::Entry>, StorageError> {
        let mut entries = self.0.list(dir).await?;
        if let Some(file) = entries
            .iter()
            .find(|e| e.kind == neo_storage::EntryKind::File)
            .cloned()
        {
            entries.push(file);
        }
        Ok(entries)
    }

    async fn stat(&self, path: &neo_storage::RawPath) -> Result<neo_storage::Entry, StorageError> {
        self.0.stat(path).await
    }

    async fn source(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Source, StorageError> {
        self.0.source(path).await
    }

    async fn access(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Access, StorageError> {
        self.0.access(path).await
    }
}

#[tokio::test]
async fn a_name_listed_twice_is_kept_once_and_the_scan_goes_on() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ01000001/01.mp3", b"one");
    lib.write("a/RJ01000002/01.mp3", b"two");
    let storage = Repeating(LocalStorage::new(lib.path("a")));
    let s = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &NoObserver,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        (s.works_found, s.tracks_new, s.works_failed),
        (2, 2, 0),
        "{s:?}"
    );
    assert!(
        s.log.iter().any(|l| l.contains("listed twice")),
        "{:?}",
        s.log
    );
}

/// A local root whose listings name every directory twice, as an OpenList
/// listing can.
struct RepeatingDirs(LocalStorage);

#[async_trait::async_trait]
impl neo_storage::Storage for RepeatingDirs {
    async fn list(
        &self,
        dir: &neo_storage::RawPath,
    ) -> Result<Vec<neo_storage::Entry>, StorageError> {
        let mut entries = self.0.list(dir).await?;
        let dirs: Vec<_> = entries
            .iter()
            .filter(|e| e.kind == neo_storage::EntryKind::Dir)
            .cloned()
            .collect();
        entries.extend(dirs);
        Ok(entries)
    }

    async fn stat(&self, path: &neo_storage::RawPath) -> Result<neo_storage::Entry, StorageError> {
        self.0.stat(path).await
    }

    async fn source(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Source, StorageError> {
        self.0.source(path).await
    }

    async fn access(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Access, StorageError> {
        self.0.access(path).await
    }
}

#[tokio::test]
async fn a_directory_listed_twice_is_one_location_and_the_scan_goes_on() {
    let lib = Lib::new(&["a"]).await;
    lib.write("a/RJ01000001/01.mp3", b"one");
    lib.write("a/series/RJ01000002/01.mp3", b"two");
    let storage = RepeatingDirs(LocalStorage::new(lib.path("a")));
    let s = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &NoObserver,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        (s.works_found, s.works_new, s.tracks_new, s.works_failed),
        (2, 2, 2, 0),
        "{s:?}"
    );
    assert_eq!(lib.i64("SELECT COUNT(*) FROM work_locations").await, 2);
    assert!(
        s.log.iter().any(|l| l.contains("listed twice")),
        "{:?}",
        s.log
    );
}

/// A local root that answers each listing after a delay, like a remote,
/// and records how many listings were in flight at once and how many ended.
struct Slow {
    inner: LocalStorage,
    delay: std::time::Duration,
    concurrency: usize,
    in_flight: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
    listed: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl neo_storage::Storage for Slow {
    fn list_concurrency(&self) -> usize {
        self.concurrency
    }

    async fn list(
        &self,
        dir: &neo_storage::RawPath,
    ) -> Result<Vec<neo_storage::Entry>, StorageError> {
        use std::sync::atomic::Ordering::SeqCst;
        let now = self.in_flight.fetch_add(1, SeqCst) + 1;
        self.peak.fetch_max(now, SeqCst);
        tokio::time::sleep(self.delay).await;
        self.in_flight.fetch_sub(1, SeqCst);
        let entries = self.inner.list(dir).await;
        self.listed.fetch_add(1, SeqCst);
        entries
    }

    async fn stat(&self, path: &neo_storage::RawPath) -> Result<neo_storage::Entry, StorageError> {
        self.inner.stat(path).await
    }

    async fn source(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Source, StorageError> {
        self.inner.source(path).await
    }

    async fn access(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Access, StorageError> {
        self.inner.access(path).await
    }
}

/// Scans a fresh copy of the same synthetic root with the given listing
/// concurrency. Returns the peak number of listings in flight and every
/// work and track ID by path. The peak, not the wall time, shows the
/// overlap: a loaded machine slows the database work that is not delayed,
/// which made a speedup bound flaky.
async fn scan_with_concurrency(concurrency: usize) -> (usize, Vec<(String, i64)>) {
    let lib = Lib::new(&["a"]).await;
    for w in 1..=8 {
        for group in ["mp3", "wav"] {
            lib.write(&format!("a/RJ0100000{w}/{group}/01.mp3"), b"one");
            lib.write(&format!("a/RJ0100000{w}/{group}/02.mp3"), b"two");
        }
    }
    let storage = Slow {
        inner: LocalStorage::new(lib.path("a")),
        delay: std::time::Duration::from_millis(40),
        concurrency,
        in_flight: Default::default(),
        peak: Default::default(),
        listed: Default::default(),
    };
    let s = scan_root(
        &lib.db,
        &Root {
            id: "a",
            storage: &storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["a".to_owned()],
        &NoObserver,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!((s.works_new, s.tracks_new), (8, 32), "{s:?}");
    let ids: Vec<(String, i64)> = testing::rows(
        &lib.db,
        "SELECT 'work ' || code, id FROM works
         UNION ALL SELECT 'track ' || display_path || ' of ' || work_id, id FROM tracks
         ORDER BY 1",
    )
    .await;
    (storage.peak.into_inner(), ids)
}

#[tokio::test]
async fn work_directories_are_listed_concurrently_with_the_same_ids() {
    let (one_peak, one_ids) = scan_with_concurrency(1).await;
    let (four_peak, four_ids) = scan_with_concurrency(4).await;
    assert_eq!(one_peak, 1);
    assert_eq!(four_peak, 4, "bounded by list_concurrency");
    assert_eq!(one_ids, four_ids, "IDs do not depend on listing order");
}

#[tokio::test]
async fn many_listings_ahead_do_not_starve_the_writes() {
    let lib = Lib::new(&["a"]).await;
    for w in 10..74 {
        lib.write(&format!("a/RJ010000{w}/01.mp3"), b"one");
    }
    let storage = Slow {
        inner: LocalStorage::new(lib.path("a")),
        delay: std::time::Duration::from_millis(1),
        concurrency: 16,
        in_flight: Default::default(),
        peak: Default::default(),
        listed: Default::default(),
    };
    let root = Root {
        id: "a",
        storage: &storage,
        probe_after_scan: false,
        skip_unchanged: true,
    };
    // The second scan compares every work with its stored digest. Each
    // comparison used to hold a read connection while it waited its turn,
    // and the writes then waited for one until the pool timed out.
    let order = ["a".to_owned()];
    for _ in 0..2 {
        let scan = scan_root(&lib.db, &root, &order, &NoObserver, NOW);
        finishes_while_listing(scan, &storage.listed)
            .await
            .expect("the writes got a connection");
    }
}

/// Awaits a scan, failing only when its listings stop for a long while. A
/// loaded machine slows the scan, sometimes past any fixed total, but a
/// starved write stops the listings behind it altogether.
async fn finishes_while_listing<T>(
    scan: impl std::future::Future<Output = T>,
    listed: &std::sync::atomic::AtomicUsize,
) -> T {
    const STALL: std::time::Duration = std::time::Duration::from_secs(60);
    tokio::pin!(scan);
    let mut seen = listed.load(std::sync::atomic::Ordering::SeqCst);
    loop {
        tokio::select! {
            outcome = &mut scan => return outcome,
            () = tokio::time::sleep(STALL) => {
                let now = listed.load(std::sync::atomic::Ordering::SeqCst);
                assert_ne!(now, seen, "no listing ended for {STALL:?}");
                seen = now;
            }
        }
    }
}

#[tokio::test]
async fn merged_old_data_attaches_when_its_work_is_created() {
    let lib = Lib::new(&["a"]).await;
    exec(
        &lib.db,
        r#"INSERT INTO users (name, is_admin, created_at) VALUES ('u1', FALSE, 0);
           INSERT INTO legacy_user_state
             (user_id, code, favorite_at, has_review, rating, progress_at, position_ms,
              pending_source, pending_state)
           VALUES
             (1, 'RJ123456', 5, TRUE, 4, 7, 12500,
              '{"item":{"title":"01.mp3","subtitle":"mp3"},"seconds":12.5}', 'pending'),
             (1, 'RJ999999', 5, FALSE, NULL, NULL, NULL, NULL, NULL);"#,
    )
    .await;
    lib.write("a/RJ123456/mp3/01.mp3", b"one");
    let s = lib.ok("a").await;

    assert!(
        s.log
            .iter()
            .any(|l| l.contains("RJ123456: merged old data")),
        "{:?}",
        s.log
    );
    assert_eq!(lib.i64("SELECT COUNT(*) FROM favorites").await, 1);
    assert_eq!(lib.i64("SELECT rating FROM legacy_reviews").await, 4);
    let (track, _, _) = lib.track("mp3/01.mp3").await;
    assert_eq!(
        lib.i64("SELECT track_id FROM play_progress").await,
        track,
        "the merged position is matched in the same scan"
    );
    assert_eq!(s.pending_matched, 1);
    // A work that was not found keeps waiting.
    assert_eq!(
        lib.i64("SELECT COUNT(*) FROM legacy_user_state WHERE code = 'RJ999999'")
            .await,
        1
    );
    assert_eq!(lib.i64("SELECT COUNT(*) FROM legacy_user_state").await, 1);
}
