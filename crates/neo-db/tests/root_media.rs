//! Root relocation invalidates derived media, including late job writes.

use neo_db::{analysis, media, roots, testing};

async fn setup() -> (testing::TestDb, roots::StoredRoot) {
    let test = testing::open().await;
    let root = roots::StoredRoot {
        id: "lib".into(),
        display_name: "Library".into(),
        kind: "local".into(),
        settings: r#"{"path":"/a"}"#.into(),
    };
    roots::insert_stored(&test.db, &root, 0).await.unwrap();
    testing::exec(&test.db,
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
         VALUES (1, 'RJ000001', 'Synthetic work', 'none', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, listing_hash)
         VALUES (1, 1, 'lib', CAST('RJ000001' AS BLOB), 'RJ000001', 'old');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key,
                             kind, size, mtime, probe, duration_ms, duration_src)
         VALUES (1, 1, 1, CAST('01.wav' AS BLOB), '01.wav', 'a', 'audio', 10, 20, '{}', 2000, 'probe');
         INSERT INTO track_analysis (track_id, size, mtime, integrated_lufs, true_peak_dbtp, analyzed_at)
         VALUES (1, 10, 20, -23, -20, 0);"
    ).await;
    (test, root)
}

#[tokio::test]
async fn relocation_clears_derived_media_but_a_rename_keeps_it() {
    let (test, mut root) = setup().await;
    let db = &test.db;
    root.display_name = "Renamed".into();
    roots::update_stored(db, &root, false, 1).await.unwrap();
    let before = media::track_file(db, 1).await.unwrap().unwrap();
    assert!(before.probe.is_some());
    assert!(
        analysis::loudness(db, 1, before.size, before.mtime, 1)
            .await
            .unwrap()
            .is_some()
    );

    root.settings = r#"{"path":"/b"}"#.into();
    roots::update_stored(db, &root, true, 2).await.unwrap();
    let after = media::track_file(db, 1).await.unwrap().unwrap();
    assert_eq!(after.probe, None);
    assert_eq!(after.duration_ms, None);
    assert!(
        analysis::loudness(db, 1, after.size, after.mtime, 2)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(after.id, before.id);
}

#[tokio::test]
async fn a_probe_from_the_old_location_cannot_write_back_after_relocation() {
    let (test, mut root) = setup().await;
    let db = &test.db;
    let old = media::track_file(db, 1).await.unwrap().unwrap();
    root.settings = r#"{"path":"/b"}"#.into();
    roots::update_stored(db, &root, true, 1).await.unwrap();
    let current = media::track_file(db, 1).await.unwrap().unwrap();
    media::save_probe(db, &current, "new probe", Some(3000))
        .await
        .unwrap();
    media::save_probe(db, &old, "old probe", Some(2000))
        .await
        .unwrap();
    let after = media::track_file(db, 1).await.unwrap().unwrap();
    assert_eq!(after.probe.as_deref(), Some("new probe"));
    assert_eq!(after.duration_ms, Some(3000));
}

#[tokio::test]
async fn an_analysis_from_the_old_location_cannot_write_back_after_relocation() {
    let (test, mut root) = setup().await;
    let db = &test.db;
    let old = media::track_file(db, 1).await.unwrap().unwrap();
    root.settings = r#"{"path":"/b"}"#.into();
    roots::update_stored(db, &root, true, 1).await.unwrap();
    let stale = analysis::StoredLoudness {
        integrated_lufs: Some(-23.0),
        true_peak_dbtp: Some(-20.0),
    };
    analysis::save(db, &old, old.size, old.mtime, stale, 2)
        .await
        .unwrap();
    assert!(
        analysis::loudness(db, 1, old.size, old.mtime, 2)
            .await
            .unwrap()
            .is_none()
    );
    let current = media::track_file(db, 1).await.unwrap().unwrap();
    let measured = analysis::StoredLoudness {
        integrated_lufs: Some(-13.0),
        true_peak_dbtp: Some(-10.0),
    };
    analysis::save(db, &current, current.size, current.mtime, measured, 3)
        .await
        .unwrap();
    analysis::save(db, &old, old.size, old.mtime, stale, 4)
        .await
        .unwrap();
    assert_eq!(
        analysis::loudness(db, 1, current.size, current.mtime, 4)
            .await
            .unwrap(),
        Some(measured)
    );
}
