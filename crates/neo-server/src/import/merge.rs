//! Merges an old library's user data into a library that already has works
//! of its own, as "合并到已有的库" in book/src/migration.md describes. Only
//! favorites, reviews and play positions come over. The old works, their
//! metadata and covers do not, because the new library scans its own.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use neo_core::review;
use neo_db::{
    self as db, Db, Tx, import,
    legacy_state::{self, Claimed, Progress, Review, State},
    scan,
};
use tokio_util::sync::CancellationToken;

use super::error::{ErrorCode, refuse};
use super::plan::Plan;
use crate::token;

/// What the merge did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// Old users that had no account of the same name, created without a
    /// password.
    pub users_created: Vec<String>,
    /// Works with old user data.
    pub works: u64,
    /// What attached to works the library already has.
    pub claimed: Claimed,
    /// Work codes still waiting for a scan, across every merge so far.
    pub waiting: u64,
}

/// Old user name to the account it merges into, from `--user OLD=NEW`.
/// Old users not named keep their name.
pub type UserMap = BTreeMap<String, String>;

/// Parses `--user OLD=NEW` pairs against the old users in `plan`.
pub fn user_map(pairs: &[String], plan: &Plan) -> Result<UserMap> {
    let mut map = UserMap::new();
    for pair in pairs {
        let Some((old, new)) = pair.rsplit_once('=') else {
            bail!("--user {pair:?} is not OLD=NEW");
        };
        if !plan.users.iter().any(|u| u.name == old) {
            bail!("--user {pair:?}: the old library has no user {old:?}");
        }
        crate::user_cmd::validate_name(new).with_context(|| format!("--user {pair:?}"))?;
        if map.insert(old.to_owned(), new.to_owned()).is_some() {
            bail!("--user names {old:?} twice");
        }
    }
    Ok(map)
}

/// Writes the plan's user data into `db` in one transaction. Rows for works
/// the library already has attach at once. The rest wait in
/// `legacy_user_state` until a scan creates the work.
/// Cancelling `cancel` before the commit rolls the whole merge back.
pub async fn merge(
    db: &Db,
    plan: &Plan,
    users: &UserMap,
    now: i64,
    cancel: &CancellationToken,
) -> Result<Merged> {
    // The CLI may merge while the server writes. A transaction that read
    // before the server committed fails with SQLITE_BUSY_SNAPSHOT however
    // long it waits, so the whole merge runs again, as retry_busy does.
    let mut retries = 0;
    loop {
        match attempt(db, plan, users, now, cancel).await {
            Err(e)
                if retries < BUSY_RETRIES
                    && e.downcast_ref::<sqlx::Error>().is_some_and(db::is_busy) =>
            {
                retries += 1;
                tokio::time::sleep(BUSY_BACKOFF * retries).await;
            }
            result => return result,
        }
    }
}

const BUSY_RETRIES: u32 = 3;
const BUSY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

async fn attempt(
    db: &Db,
    plan: &Plan,
    users: &UserMap,
    now: i64,
    cancel: &CancellationToken,
) -> Result<Merged> {
    let states = states(plan);
    let mut merged = Merged {
        works: states_len(plan) as u64,
        ..Merged::default()
    };
    // No other writer can commit between the reads and the writes.
    let mut tx = db.begin_exclusive().await?;
    let mut user_ids = BTreeMap::new();
    // Several old users may merge into one account, so states are combined
    // by account before any is stored.
    let mut combined = BTreeMap::<(i64, String), State>::new();
    for ((user, work), state) in states {
        let user_id = match user_ids.get(&user) {
            Some(&id) => id,
            None => {
                let old = &plan.users[user].name;
                let name = users.get(old).unwrap_or(old);
                let id = account(&mut tx, name, now, &mut merged.users_created).await?;
                user_ids.insert(user, id);
                id
            }
        };
        let code = plan.works[work].code.clone();
        match combined.entry((user_id, code.clone())) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(State {
                    user_id,
                    code,
                    ..state
                });
            }
            std::collections::btree_map::Entry::Occupied(mut e) => combine(e.get_mut(), state),
        }
    }
    for state in combined.values() {
        legacy_state::put(&mut tx, state).await?;
    }
    let codes = import::waiting_codes(&mut tx).await?;
    for code in codes {
        if let Some(work_id) = scan::work_by_code(&mut tx, &code).await? {
            let c = legacy_state::claim(&mut tx, work_id, &code).await?;
            merged.claimed.favorites += c.favorites;
            merged.claimed.reviews += c.reviews;
            merged.claimed.progress += c.progress;
            merged.claimed.progress_kept += c.progress_kept;
        }
    }
    merged.waiting = import::waiting_count(&mut tx)
        .await?
        .try_into()
        .unwrap_or(0);
    if cancel.is_cancelled() {
        // Dropping the transaction rolls it back. The library is untouched.
        refuse!(
            ErrorCode::Cancelled,
            "the merge was cancelled; the library is unchanged"
        );
    }
    tx.commit().await?;
    Ok(merged)
}

/// Folds a second old user's state for the same account and work into
/// `into`, with the rules `claim` uses against the library: the earlier
/// favorite, the newer position, and the review that says more.
fn combine(into: &mut State, from: State) {
    into.favorite_at = match (into.favorite_at, from.favorite_at) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    // Counted on the review as it will be stored: old kikoeru saved empty
    // texts and unchecked ratings, which `claim` drops.
    let says = |r: &Review| {
        review::from_legacy(r.rating, r.review_text.as_deref())
            .review
            .map_or(0, |v| {
                u8::from(v.rating.is_some()) + u8::from(v.text.is_some())
            })
    };
    if let Some(review) = from.review
        && into.review.as_ref().is_none_or(|r| says(&review) > says(r))
    {
        into.review = Some(review);
    }
    if let Some(progress) = from.progress
        && into
            .progress
            .as_ref()
            .is_none_or(|p| progress.updated_at > p.updated_at)
    {
        into.progress = Some(progress);
    }
}

/// How many works have old user data.
pub fn states_len(plan: &Plan) -> usize {
    let works: std::collections::BTreeSet<usize> = states(plan).keys().map(|(_, w)| *w).collect();
    works.len()
}

/// The plan's favorites, reviews and positions, grouped by old user and
/// work. `user_id` and `code` are filled in by the caller.
fn states(plan: &Plan) -> BTreeMap<(usize, usize), State> {
    let mut states = BTreeMap::<(usize, usize), State>::new();
    for &(user, work, at) in &plan.favorites {
        states.entry((user, work)).or_default().favorite_at = Some(at);
    }
    for r in &plan.reviews {
        states.entry((r.user, r.work)).or_default().review = Some(Review {
            rating: r.rating,
            review_text: r.review_text.clone(),
            progress: r.progress.clone(),
            updated_at: r.updated_at,
        });
    }
    for p in &plan.progress {
        states.entry((p.user, p.work)).or_default().progress = Some(Progress {
            updated_at: p.updated_at,
            position_ms: p.position_ms,
            finished_at: p.finished_at,
            pending: None,
        });
    }
    // Every pending row comes from a play history that also made a
    // progress row, and only matters together with it.
    for p in &plan.pending {
        if let Some(progress) = states
            .get_mut(&(p.user, p.work))
            .and_then(|s| s.progress.as_mut())
        {
            progress.pending = Some((p.source.clone(), p.durations.clone(), p.state.to_owned()));
        }
    }
    states
}

/// The account `name`, created without a password when it does not exist.
/// A created account is never an administrator, because the library
/// already has its own.
async fn account(tx: &mut Tx<'_>, name: &str, now: i64, created: &mut Vec<String>) -> Result<i64> {
    if let Some(id) = import::user_id(tx, name).await? {
        return Ok(id);
    }
    let version = token::initial_version()?;
    let id = import::insert_user(tx, name, false, version, now)
        .await
        .with_context(|| format!("creating user {name}"))?;
    created.push(name.to_owned());
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(at: i64) -> Option<Progress> {
        Some(Progress {
            updated_at: at,
            position_ms: at,
            finished_at: None,
            pending: None,
        })
    }

    #[test]
    fn combining_keeps_the_earlier_favorite_newer_position_and_fuller_review() {
        let mut into = State {
            favorite_at: Some(9),
            review: Some(Review {
                progress: Some("marked".into()),
                ..Review::default()
            }),
            progress: progress(5),
            ..State::default()
        };
        combine(
            &mut into,
            State {
                favorite_at: Some(3),
                review: Some(Review {
                    rating: Some(4),
                    ..Review::default()
                }),
                progress: progress(7),
                ..State::default()
            },
        );
        assert_eq!(into.favorite_at, Some(3));
        assert_eq!(into.review.as_ref().unwrap().rating, Some(4));
        assert_eq!(into.progress.as_ref().unwrap().updated_at, 7);

        combine(&mut into, State::default());
        assert_eq!(
            into.favorite_at,
            Some(3),
            "nothing is lost to an empty state"
        );
        assert!(into.review.is_some() && into.progress.is_some());
        combine(
            &mut into,
            State {
                progress: progress(1),
                ..State::default()
            },
        );
        assert_eq!(
            into.progress.unwrap().updated_at,
            7,
            "an older position loses"
        );
    }
}
