//! Writes a plan into a freshly migrated database. Everything goes into one
//! transaction, so a failure leaves no partial library behind.

use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

use anyhow::{Context, Result, bail};
use neo_core::review;
use neo_db::{
    Db, Tx,
    import::{self as store, ImportedWork},
    reviews, scan,
};

use super::error::{ErrorCode, refuse};
use super::plan::Plan;
use tokio_util::sync::CancellationToken;

use crate::{private::PrivateFile, token};

/// How many rows or covers go by between two looks at the cancel token.
const CHECK_EVERY: usize = 200;

/// Counts written rows and stops the import once it is cancelled. The
/// transaction is dropped on the error, so nothing of it is committed.
struct Ticker<'a> {
    cancel: &'a CancellationToken,
    n: usize,
}

impl Ticker<'_> {
    fn tick(&mut self) -> Result<()> {
        self.n += 1;
        if self.n.is_multiple_of(CHECK_EVERY) {
            #[cfg(debug_assertions)]
            pause_for_tests();
            self.check()?;
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            refuse!(ErrorCode::Cancelled, "the import was cancelled");
        }
        Ok(())
    }
}

/// Debug builds only: `KIKOERU_TEST_IMPORT_PAUSE_MS` slows every check, so
/// a test can signal an import that is surely still writing instead of
/// racing its end.
#[cfg(debug_assertions)]
fn pause_for_tests() {
    let pause = std::env::var("KIKOERU_TEST_IMPORT_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok());
    if let Some(ms) = pause {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// Inserts the plan and copies the covers. Returns the new work IDs in the
/// order of `plan.works`.
pub async fn write(
    db: &Db,
    plan: &Plan,
    covers_dir: &Path,
    now: i64,
    cancel: &CancellationToken,
) -> Result<Vec<i64>> {
    let mut ticker = Ticker { cancel, n: 0 };
    ticker.check()?;
    let mut tx = db.begin().await?;
    for (id, display_name) in &plan.roots {
        store::insert_root(&mut tx, id, display_name).await?;
    }
    dictionaries(&mut tx, plan).await?;
    let work_ids = works(&mut tx, plan, &mut ticker).await?;
    let user_ids = users(&mut tx, plan, now).await?;
    user_state(&mut tx, plan, &work_ids, &user_ids, &mut ticker).await?;
    for (work, id) in plan.works.iter().zip(&work_ids) {
        ticker.tick()?;
        if let Some(src) = &work.cover {
            copy_cover(src, &covers_dir.join(format!("{id}.jpg")))?;
        }
    }
    // Batches of 200 can end between two checks, so the last look is
    // right before the commit.
    ticker.check()?;
    tx.commit().await?;
    Ok(work_ids)
}

async fn dictionaries(tx: &mut Tx<'_>, plan: &Plan) -> Result<()> {
    for (id, name) in &plan.circles {
        store::insert_circle(tx, *id, name).await?;
    }
    for (id, name) in &plan.vas {
        store::insert_va(tx, id, name).await?;
    }
    for (id, name) in &plan.tags {
        store::insert_tag(tx, *id, name).await?;
    }
    Ok(())
}

async fn works(tx: &mut Tx<'_>, plan: &Plan, ticker: &mut Ticker<'_>) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(plan.works.len());
    for w in &plan.works {
        ticker.tick()?;
        let cover_src = if w.cover.is_some() {
            "imported"
        } else {
            "none"
        };
        let work = ImportedWork {
            code: &w.code,
            title: &w.title,
            circle_id: w.circle_id,
            release_date: w.release_date.as_deref(),
            nsfw: w.nsfw,
            dl_count: w.dl_count,
            price: w.price,
            rate_average: w.rate_average,
            rate_count: w.rate_count,
            metadata_src: w.metadata_src,
            cover_src,
            extra: &w.extra,
            created_at: w.created_at,
            updated_at: w.updated_at,
        };
        let id = store::insert_work(tx, work)
            .await
            .with_context(|| format!("inserting work {}", w.code))?;
        // Old paths came from JavaScript strings, so they are valid UTF-8
        // and the raw bytes are the UTF-8 encoding.
        let rel_path = w.rel_path.as_bytes();
        scan::insert_location(tx, id, &w.root_id, rel_path, &w.rel_path)
            .await
            .with_context(|| format!("inserting the location of work {}", w.code))?;
        ids.push(id);
    }
    for (work, tag_id) in &plan.work_tags {
        ticker.tick()?;
        store::insert_work_tag(tx, ids[*work], *tag_id).await?;
    }
    for (work, va_id) in &plan.work_vas {
        store::insert_work_va(tx, ids[*work], va_id).await?;
    }
    Ok(ids)
}

/// Old password hashes are not kept (book/src/auth.md), so every user must
/// get a new password from `user set-password` before logging in.
async fn users(tx: &mut Tx<'_>, plan: &Plan, now: i64) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(plan.users.len());
    for u in &plan.users {
        let version = token::initial_version()?;
        let id = store::insert_user(tx, &u.name, u.is_admin, version, now)
            .await
            .with_context(|| format!("inserting user {}", u.name))?;
        ids.push(id);
    }
    Ok(ids)
}

async fn user_state(
    tx: &mut Tx<'_>,
    plan: &Plan,
    works: &[i64],
    users: &[i64],
    ticker: &mut Ticker<'_>,
) -> Result<()> {
    for (user, work, created_at) in &plan.favorites {
        ticker.tick()?;
        store::insert_favorite(tx, users[*user], works[*work], *created_at).await?;
    }
    for p in &plan.progress {
        ticker.tick()?;
        let (user_id, work_id) = (users[p.user], works[p.work]);
        store::insert_progress(
            tx,
            user_id,
            work_id,
            p.position_ms,
            p.finished_at,
            p.updated_at,
        )
        .await?;
    }
    for r in &plan.reviews {
        ticker.tick()?;
        let (user_id, work_id) = (users[r.user], works[r.work]);
        store::insert_legacy_review(
            tx,
            user_id,
            work_id,
            r.rating,
            r.review_text.as_deref(),
            r.progress.as_deref(),
        )
        .await?;
        if let Some(valid) = review::from_legacy(r.rating, r.review_text.as_deref()).review {
            reviews::insert_legacy(tx, user_id, work_id, &valid, r.updated_at).await?;
        }
    }
    for p in &plan.pending {
        ticker.tick()?;
        let (user_id, work_id) = (users[p.user], works[p.work]);
        store::insert_pending(
            tx,
            user_id,
            work_id,
            &p.source,
            p.durations.as_deref(),
            p.state,
        )
        .await?;
    }
    Ok(())
}

/// Copies into a new 0600 file. `fs::copy` would carry over the source's
/// mode, and old covers can be world-writable.
fn copy_cover(src: &Path, dst: &Path) -> Result<()> {
    let mut from = File::open(src).with_context(|| format!("cannot read {}", src.display()))?;
    let mut to = OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(dst)
        .with_context(|| format!("cannot create {}", dst.display()))?;
    io::copy(&mut from, &mut to).with_context(|| format!("cannot copy {}", src.display()))?;
    Ok(())
}

/// The structure check that must pass before the directory is used as a
/// data directory: SQLite's own integrity check, no dangling references,
/// and the row counts the plan promised.
pub async fn verify(db: &Db, plan: &Plan, expected: &[(&'static str, u64)]) -> Result<()> {
    if let Some(sqlite) = db.sqlite() {
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&sqlite.read)
            .await?;
        if integrity != "ok" {
            bail!("integrity check failed: {integrity}");
        }
        let dangling: Vec<(String,)> =
            sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
                .fetch_all(&sqlite.read)
                .await?;
        if !dangling.is_empty() {
            bail!(
                "{} rows break a foreign key, first in {}",
                dangling.len(),
                dangling[0].0
            );
        }
    }
    for (table, want) in expected {
        // The table names come from the fixed list in the report, never
        // from input.
        let got = db.count_rows(table).await?;
        if u64::try_from(got).ok() != Some(*want) {
            bail!("{table} has {got} rows, expected {want}");
        }
    }
    let covers = plan.works.iter().filter(|w| w.cover.is_some()).count();
    let imported = store::imported_covers(db).await?;
    if usize::try_from(imported).ok() != Some(covers) {
        bail!("{imported} works have an imported cover, expected {covers}");
    }
    Ok(())
}
