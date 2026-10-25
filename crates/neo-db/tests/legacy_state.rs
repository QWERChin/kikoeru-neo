use neo_db::{
    Db,
    legacy_state::{self, Claimed, Progress, Review, State},
    testing::{self, TestDb, exec},
};

async fn open() -> (TestDb, Db) {
    let t = testing::open().await;
    let db = t.db.clone();
    exec(
        &db,
        "INSERT INTO users (name, is_admin, created_at) VALUES ('u', FALSE, 0);
         INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ('RJ1', 'a', 'none', 0, 0), ('RJ2', 'b', 'none', 0, 0);",
    )
    .await;
    (t, db)
}

fn state(code: &str, progress_at: i64) -> State {
    State {
        user_id: 1,
        code: code.to_owned(),
        favorite_at: Some(3),
        review: Some(Review {
            rating: Some(5),
            review_text: Some("old".into()),
            progress: Some("marked".into()),
            updated_at: Some(7),
        }),
        progress: Some(Progress {
            updated_at: progress_at,
            position_ms: 1000,
            finished_at: None,
            pending: Some(("{}".into(), None, "pending".into())),
        }),
    }
}

/// work_id, rating, text and updated_at of a `reviews` row.
type ReviewRow = (i64, Option<i64>, Option<String>, Option<i64>);

async fn i64(db: &Db, sql: &'static str) -> i64 {
    testing::value(db, sql).await
}

#[tokio::test]
async fn claim_keeps_what_the_user_already_has() {
    let (_dir, db) = open().await;
    // The library already has a favorite and a review for RJ1, and a newer
    // position. RJ2 has an older position.
    exec(
        &db,
        "INSERT INTO favorites (user_id, work_id, created_at) VALUES (1, 1, 99);
         INSERT INTO legacy_reviews (user_id, work_id, rating) VALUES (1, 1, 1);
         INSERT INTO reviews (user_id, work_id, rating, text, updated_at)
           VALUES (1, 1, 2, 'mine', 60);
         INSERT INTO play_progress (user_id, work_id, track_id, position_ms, updated_at)
           VALUES (1, 1, NULL, 5, 50), (1, 2, NULL, 5, 5);",
    )
    .await;
    let mut tx = db.begin().await.unwrap();
    legacy_state::put(&mut tx, &state("RJ1", 10)).await.unwrap();
    legacy_state::put(&mut tx, &state("RJ2", 10)).await.unwrap();
    let one = legacy_state::claim(&mut tx, 1, "RJ1").await.unwrap();
    let two = legacy_state::claim(&mut tx, 2, "RJ2").await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        one,
        Claimed {
            favorites: 0,
            reviews: 0,
            progress: 0,
            progress_kept: 1
        }
    );
    assert_eq!(
        two,
        Claimed {
            favorites: 1,
            reviews: 1,
            progress: 1,
            progress_kept: 0
        }
    );
    assert_eq!(
        i64(&db, "SELECT created_at FROM favorites WHERE work_id = 1").await,
        99
    );
    assert_eq!(
        i64(&db, "SELECT rating FROM legacy_reviews WHERE work_id = 1").await,
        1
    );
    let shown: Vec<ReviewRow> = testing::rows(
        &db,
        "SELECT work_id, rating, text, updated_at FROM reviews ORDER BY work_id",
    )
    .await;
    assert_eq!(
        shown,
        [
            (1, Some(2), Some("mine".into()), Some(60)),
            (2, Some(5), Some("old".into()), Some(7)),
        ],
        "the user's own review stays, the old one fills the gap"
    );
    assert_eq!(
        i64(
            &db,
            "SELECT position_ms FROM play_progress WHERE work_id = 1"
        )
        .await,
        5
    );
    assert_eq!(
        i64(
            &db,
            "SELECT position_ms FROM play_progress WHERE work_id = 2"
        )
        .await,
        1000,
        "the old position was newer"
    );
    assert_eq!(
        i64(&db, "SELECT COUNT(*) FROM import_pending WHERE work_id = 2").await,
        1
    );
    assert_eq!(i64(&db, "SELECT COUNT(*) FROM import_pending").await, 1);
    assert_eq!(i64(&db, "SELECT COUNT(*) FROM legacy_user_state").await, 0);
}

#[tokio::test]
async fn a_second_merge_replaces_the_waiting_rows() {
    let (_dir, db) = open().await;
    let mut tx = db.begin().await.unwrap();
    legacy_state::put(&mut tx, &state("RJ9", 10)).await.unwrap();
    legacy_state::put(&mut tx, &state("RJ9", 20)).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        i64(&db, "SELECT progress_at FROM legacy_user_state").await,
        20
    );
}

#[tokio::test]
async fn a_replaced_position_drops_the_old_matching_material() {
    let (_dir, db) = open().await;
    exec(
        &db,
        "INSERT INTO play_progress (user_id, work_id, track_id, position_ms, updated_at)
           VALUES (1, 1, NULL, 5, 5);
         INSERT INTO import_pending (user_id, work_id, source, state)
           VALUES (1, 1, '{\"item\":{\"title\":\"old.mp3\"}}', 'pending');",
    )
    .await;
    let mut state = state("RJ1", 10);
    state.progress.as_mut().unwrap().pending = None;
    let mut tx = db.begin().await.unwrap();
    legacy_state::put(&mut tx, &state).await.unwrap();
    legacy_state::claim(&mut tx, 1, "RJ1").await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(i64(&db, "SELECT updated_at FROM play_progress").await, 10);
    assert_eq!(i64(&db, "SELECT COUNT(*) FROM import_pending").await, 0);
}
