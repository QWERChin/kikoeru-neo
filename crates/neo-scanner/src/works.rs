//! Turns the discovered directories into works and locations, and marks
//! the locations of this root that were not found as missing.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use neo_db::{
    Db, Tx,
    legacy_state::{self, Claimed},
    scan,
};
use neo_storage::RawPath;

use crate::{Summary, discover::Found};

/// Short write transactions, per book/src/scanning.md.
const BATCH: usize = 200;

/// A location of this root that exists on disk after the scan.
pub struct Present {
    pub work_id: i64,
    pub location_id: i64,
    pub rel_path: RawPath,
    pub display: String,
}

struct Existing {
    id: i64,
    work_id: i64,
    missing: bool,
}

pub async fn sync(
    db: &Db,
    root_id: &str,
    found: &[Found],
    now: i64,
    summary: &mut Summary,
) -> Result<(Vec<Present>, BTreeSet<i64>), sqlx::Error> {
    let rows = scan::root_locations(db, root_id).await?;
    let existing: HashMap<Vec<u8>, Existing> = rows
        .into_iter()
        .map(|r| {
            (
                r.rel_path,
                Existing {
                    id: r.id,
                    work_id: r.work_id,
                    missing: r.missing,
                },
            )
        })
        .collect();

    let mut present = Vec::with_capacity(found.len());
    for batch in found.chunks(BATCH) {
        let mut tx = db.begin().await?;
        for f in batch {
            let rel = f.rel_path.as_bytes();
            let (work_id, location_id) = match existing.get(rel) {
                Some(e) => {
                    if e.missing {
                        scan::restore_location(&mut tx, e.id).await?;
                        summary.locations_restored += 1;
                    }
                    (e.work_id, e.id)
                }
                None => {
                    let work_id = match scan::work_by_code(&mut tx, &f.code).await? {
                        Some(id) => {
                            summary.locations_new += 1;
                            summary.log(format!("{} is another location of {}", f.display, f.code));
                            id
                        }
                        None => {
                            // Without metadata the title is the directory
                            // name until a metadata fetch replaces it.
                            let title = f.display.rsplit('/').next().unwrap_or(&f.display);
                            summary.works_new += 1;
                            let id = scan::insert_work(&mut tx, &f.code, title, now).await?;
                            // Old user data merged before this work existed
                            // attaches now, before matching runs on it.
                            let c = legacy_state::claim(&mut tx, id, &f.code).await?;
                            if c != Claimed::default() {
                                summary.log(format!(
                                    "{}: merged old data: {} favorites, {} reviews, {} positions",
                                    f.code, c.favorites, c.reviews, c.progress
                                ));
                            }
                            id
                        }
                    };
                    let location_id =
                        scan::insert_location(&mut tx, work_id, root_id, rel, &f.display).await?;
                    (work_id, location_id)
                }
            };
            present.push(Present {
                work_id,
                location_id,
                rel_path: f.rel_path.clone(),
                display: f.display.clone(),
            });
        }
        tx.commit().await?;
    }

    let seen: BTreeSet<i64> = present.iter().map(|p| p.location_id).collect();
    let mut affected: BTreeSet<i64> = present.iter().map(|p| p.work_id).collect();
    // Locations of this root that were not found, grouped by work so each
    // batch updates a work's locations and its own missing state together.
    let mut lost: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for e in existing.values() {
        if seen.contains(&e.id) {
            continue;
        }
        affected.insert(e.work_id);
        if !e.missing {
            lost.entry(e.work_id).or_default().push(e.id);
        }
    }
    let works: Vec<i64> = affected.iter().copied().collect();
    for batch in works.chunks(BATCH) {
        let mut tx = db.begin().await?;
        for &work_id in batch {
            for &location_id in lost.get(&work_id).map(Vec::as_slice).unwrap_or_default() {
                scan::mark_location_missing(&mut tx, location_id, now).await?;
                summary.locations_missing += 1;
            }
            update_work_state(&mut tx, work_id, now, summary).await?;
        }
        tx.commit().await?;
    }
    Ok((present, affected))
}

/// A work is missing only when every location, in any root, is.
async fn update_work_state(
    tx: &mut Tx<'_>,
    work_id: i64,
    now: i64,
    summary: &mut Summary,
) -> Result<(), sqlx::Error> {
    let state = scan::work_state(tx, work_id).await?;
    if state.missing && state.present {
        scan::set_work_missing(tx, work_id, None).await?;
        summary.works_restored += 1;
    } else if !state.missing && !state.present {
        scan::set_work_missing(tx, work_id, Some(now)).await?;
        summary.works_missing += 1;
    }
    Ok(())
}
