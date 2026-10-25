//! Turns the old rows into the rows the new database gets, without touching
//! either database. The dry run prints the report of this plan and the
//! import writes it, so their numbers cannot drift apart. The rules are in
//! book/src/migration.md.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
};

use neo_core::date::is_calendar_date;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use super::{
    legacy::{Durations, History, Legacy, Review, Work},
    report::{Issue, Report},
};

/// The old ID packs the prefix into the number: `prefix index * 10^12 +
/// digits`, in this order (kikoeru-express `filesystem/idConverter.ts`).
const PREFIXES: [&str; 4] = ["RJ", "BJ", "VJ", "CC"];
const ID_SPLITTER: i64 = 1_000_000_000_000;

pub const STATE_PENDING: &str = "pending";
pub const STATE_UNMATCHED: &str = "unmatched";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub prefix: &'static str,
    pub digits: i64,
}

impl Code {
    pub fn from_legacy_id(id: i64) -> Option<Self> {
        let prefix = PREFIXES.get(usize::try_from(id / ID_SPLITTER).ok()?)?;
        let digits = id % ID_SPLITTER;
        (digits > 0).then_some(Self { prefix, digits })
    }

    /// Numbers from 1000000 on have 8 digits, older ones 6, as DLsite and
    /// the old `formatID` write them.
    pub fn width(&self) -> usize {
        if self.digits >= 1_000_000 { 8 } else { 6 }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{:0w$}", self.prefix, self.digits, w = self.width())
    }
}

/// Every `XX123456`-style code in a directory path, in order.
pub fn codes_in_path(path: &str) -> Vec<(String, i64)> {
    let bytes = path.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let prefix = path
            .get(i..i + 2)
            .map(str::to_ascii_uppercase)
            .filter(|p| PREFIXES.contains(&p.as_str()));
        let digits_len = bytes[i + 2..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        match prefix {
            Some(p) if digits_len > 0 => {
                if let Ok(n) = path[i + 2..i + 2 + digits_len].parse() {
                    found.push((p, n));
                }
                i += 2 + digits_len;
            }
            _ => i += 1,
        }
    }
    found
}

#[derive(Debug, Clone)]
pub struct WorkRow {
    pub legacy_id: i64,
    pub code: String,
    pub title: String,
    pub circle_id: Option<i64>,
    pub release_date: Option<String>,
    pub nsfw: bool,
    pub dl_count: Option<i64>,
    pub price: Option<i64>,
    pub rate_average: Option<f64>,
    pub rate_count: Option<i64>,
    pub metadata_src: &'static str,
    pub extra: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub root_id: String,
    pub rel_path: String,
    /// The memo's `duration` object: path relative to the work directory
    /// to seconds.
    pub durations: Option<Value>,
    /// The old `_img_main.jpg`, when the source has one.
    pub cover: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub name: String,
    pub is_admin: bool,
}

/// Rows that point at a work or a user do so by index into `Plan::works`
/// and `Plan::users`. The real IDs exist only after the insert.
#[derive(Debug, Clone)]
pub struct ProgressRow {
    pub user: usize,
    pub work: usize,
    pub position_ms: i64,
    pub finished_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct PendingRow {
    pub user: usize,
    pub work: usize,
    pub source: String,
    pub durations: Option<String>,
    pub state: &'static str,
}

#[derive(Debug, Clone)]
pub struct ReviewRow {
    pub user: usize,
    pub work: usize,
    pub rating: Option<i64>,
    pub review_text: Option<String>,
    pub progress: Option<String>,
    /// The old row's update time, when it is valid.
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// New root ID and the old `root_folder` name it replaces.
    pub roots: Vec<(String, String)>,
    pub circles: Vec<(i64, String)>,
    pub vas: Vec<(String, String)>,
    pub tags: Vec<(i64, String)>,
    pub works: Vec<WorkRow>,
    pub work_tags: Vec<(usize, i64)>,
    pub work_vas: Vec<(usize, String)>,
    pub users: Vec<UserRow>,
    pub favorites: Vec<(usize, usize, i64)>,
    pub progress: Vec<ProgressRow>,
    pub reviews: Vec<ReviewRow>,
    pub pending: Vec<PendingRow>,
}

/// Fatal problems: the import would write a wrong or incomplete library, so
/// it must not start. Each entry is one line for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub Vec<String>);

/// Builds the plan. `roots` maps old `root_folder` names to new root IDs.
/// Without it (a dry run that has not been given `--root`), locations keep
/// the old name and the plan cannot be written. `now` stands in for missing
/// or unreadable timestamps.
pub fn build(
    legacy: &Legacy,
    roots: Option<&BTreeMap<String, String>>,
    now: i64,
) -> (Plan, Report, Result<(), Refusal>) {
    let backslash = windows_dirs(legacy);
    let converted;
    let legacy = if backslash {
        converted = Legacy {
            works: legacy
                .works
                .iter()
                .map(|w| Work {
                    dir: w.dir.replace('\\', "/"),
                    // The memo's duration keys are the same relative paths,
                    // with each `\\` escaped as `\\\\` in its JSON text.
                    memo: w.memo.as_ref().map(|m| m.replace(r"\\", "/")),
                    ..w.clone()
                })
                .collect(),
            // t_track keys are the same paths and override the memo, so
            // both have to use the same separator.
            track_durations: legacy
                .track_durations
                .iter()
                .map(|(work, d)| {
                    let d = d.iter().map(|(p, v)| (p.replace('\\', "/"), v.clone()));
                    (*work, d.collect())
                })
                .collect(),
            ..legacy.clone()
        };
        &converted
    } else {
        legacy
    };
    let mut b = Builder {
        plan: Plan::default(),
        report: Report::new(legacy.last_migration.clone()),
        refusals: Vec::new(),
        now,
        circle_ids: HashMap::new(),
        backslash,
    };
    b.report.preflight.backslash_dirs = backslash;
    b.roots(legacy, roots);
    b.copy_dictionaries(legacy);
    let work_index = b.works(legacy, roots);
    b.relations(legacy, &work_index);
    let user_index = b.users(legacy);
    b.user_state(legacy, &work_index, &user_index);
    // The old translation queue is not imported (it is empty in practice).
    let tasks = u64::try_from(legacy.translate_tasks).unwrap_or(0);
    b.report.count_source("t_translate_task", tasks);
    b.report.skip_n("t_translate_task", tasks, None);
    b.report.set_written(&b.plan);
    let outcome = if b.refusals.is_empty() {
        Ok(())
    } else {
        Err(Refusal(b.refusals))
    };
    (b.plan, b.report, outcome)
}

struct Builder {
    plan: Plan,
    report: Report,
    refusals: Vec<String>,
    now: i64,
    /// Old circle ID to the ID it is written under.
    circle_ids: HashMap<i64, i64>,
    /// The old library came from Windows and wrote `\` between folders,
    /// in work directories and in play queue items alike.
    backslash: bool,
}

impl Builder {
    fn roots(&mut self, legacy: &Legacy, roots: Option<&BTreeMap<String, String>>) {
        let mut counts = BTreeMap::<String, u64>::new();
        for w in &legacy.works {
            *counts.entry(w.root_folder.clone()).or_default() += 1;
        }
        self.report.preflight.root_folders = counts.clone();
        let Some(roots) = roots else {
            return;
        };
        let unmapped: Vec<_> = counts.keys().filter(|k| !roots.contains_key(*k)).collect();
        if !unmapped.is_empty() {
            self.refusals.push(format!(
                "no --root mapping for root_folder {}",
                quote_list(unmapped)
            ));
        }
        let unknown: Vec<_> = roots.keys().filter(|k| !counts.contains_key(*k)).collect();
        if !unknown.is_empty() {
            self.refusals.push(format!(
                "--root names {} but no work uses that root_folder",
                quote_list(unknown)
            ));
        }
        let mut seen = BTreeSet::new();
        for (old, new) in roots {
            if seen.insert(new.clone()) {
                self.plan.roots.push((new.clone(), old.clone()));
            }
        }
    }

    fn copy_dictionaries(&mut self, legacy: &Legacy) {
        self.report.count_source("t_circle", len(&legacy.circles));
        self.report.count_source("t_va", len(&legacy.vas));
        self.report.count_source("t_tag", len(&legacy.tags));
        for (old, name) in &legacy.circles {
            let new = circle_id(*old);
            if new != *old {
                self.report.issue(Issue::new(
                    format!("t_circle id={old}"),
                    format!("custom circle gets ID {new}, because API IDs must be positive"),
                ));
            }
            self.circle_ids.insert(*old, new);
            self.plan.circles.push((new, name.clone()));
        }
        self.plan.vas = legacy.vas.clone();
        self.plan.tags = legacy.tags.clone();
        self.report.import_n("t_circle", len(&legacy.circles));
        self.report.import_n("t_va", len(&legacy.vas));
        self.report.import_n("t_tag", len(&legacy.tags));
    }

    /// Returns the old work ID to its index in `plan.works`.
    fn works(
        &mut self,
        legacy: &Legacy,
        roots: Option<&BTreeMap<String, String>>,
    ) -> HashMap<i64, usize> {
        let mut index = HashMap::new();
        let mut codes = HashMap::<String, i64>::new();
        let mut locations = HashMap::<(String, String), i64>::new();
        self.report.count_source("t_work", len(&legacy.works));
        for w in &legacy.works {
            let at = format!("t_work id={}", w.id);
            let Some(code) = Code::from_legacy_id(w.id) else {
                self.report.skip(
                    "t_work",
                    Issue::new(at, "ID does not decode to a work code"),
                );
                continue;
            };
            if !is_relative_inside(&w.dir) {
                self.report.skip(
                    "t_work",
                    Issue::new(
                        at,
                        format!(
                            "directory {:?} is not a relative path inside its root",
                            w.dir
                        ),
                    ),
                );
                continue;
            }
            let code_str = code.to_string();
            *self
                .report
                .preflight
                .codes
                .entry(format!("{}/{}", code.prefix, code.width()))
                .or_default() += 1;
            self.check_dir(w, &code, &at);
            if let Some(first) = codes.insert(code_str.clone(), w.id) {
                self.refusals.push(format!(
                    "works {first} and {} both have code {code_str}",
                    w.id
                ));
            }
            let root_id = roots
                .and_then(|r| r.get(&w.root_folder).cloned())
                .unwrap_or_else(|| w.root_folder.clone());
            // Two spellings of one name, such as NFC and the NFD a Mac
            // writes, are one folder on disk.
            let key = (root_id.clone(), w.dir.nfc().collect::<String>());
            if let Some(first) = locations.insert(key, w.id) {
                self.report.preflight.duplicate_locations += 1;
                self.refusals.push(format!(
                    "works {first} and {} share the directory {:?} in root {root_id}",
                    w.id, w.dir
                ));
            }
            let tracks = legacy.track_durations.get(&w.id);
            let row = self.work_row(w, tracks, code_str, root_id, &at);
            index.insert(w.id, self.plan.works.len());
            self.plan.works.push(row);
            self.report.import("t_work");
        }
        index
    }

    fn check_dir(&mut self, w: &Work, code: &Code, at: &str) {
        let found = codes_in_path(&w.dir);
        if found.is_empty() {
            self.report.preflight.unparsable_dirs += 1;
            self.report.issue(Issue::new(
                at,
                format!("directory {:?} has no work code", w.dir),
            ));
        } else if !found
            .iter()
            .any(|(p, n)| p == code.prefix && *n == code.digits)
        {
            self.report.preflight.dir_code_differs += 1;
            self.report.issue(Issue::new(
                at,
                format!("directory {:?} does not contain code {code}", w.dir),
            ));
        } else if !w.dir.to_ascii_uppercase().contains(&code.to_string()) {
            // The right number with a nonstandard width, such as RJ0123456.
            self.report.preflight.dir_code_nonstandard += 1;
        }
    }

    fn work_row(
        &mut self,
        w: &Work,
        tracks: Option<&Durations>,
        code: String,
        root_id: String,
        at: &str,
    ) -> WorkRow {
        let circle_id = if let Some(&id) = self.circle_ids.get(&w.circle_id) {
            Some(id)
        } else {
            self.report.issue(Issue::new(
                at,
                format!(
                    "circle {} does not exist; imported without a circle",
                    w.circle_id
                ),
            ));
            None
        };
        let release_date = w.release.clone().filter(|r| {
            let ok = is_calendar_date(r);
            if !ok {
                self.report.issue(Issue::new(
                    at,
                    format!("release {r:?} is not a YYYY-MM-DD date that exists; dropped"),
                ));
            }
            ok
        });
        let mut extra = Map::new();
        for (key, raw) in [
            ("rank", &w.rank),
            ("rate_count_detail", &w.rate_count_detail),
        ] {
            match raw.as_deref().map(serde_json::from_str::<Value>) {
                None | Some(Ok(Value::Null)) => {}
                Some(Ok(Value::Array(a))) if a.is_empty() => {}
                Some(Ok(v)) => {
                    extra.insert(key.to_owned(), v);
                }
                Some(Err(_)) => self
                    .report
                    .issue(Issue::new(at, format!("{key} is not JSON; dropped"))),
            }
        }
        // Track durations from the old memo. The scanner uses them as the
        // initial duration_ms of every track, not only of tracks that have
        // an old play position, because OpenList files are only probed when
        // first played.
        let durations = self.durations(w, tracks, at);
        if let Some(d) = durations
            .as_ref()
            .filter(|d| d.as_object().is_some_and(|o| !o.is_empty()))
        {
            extra.insert("legacy_durations".to_owned(), d.clone());
        }
        let created_at = self.time(w.created_at, at, "created_at");
        WorkRow {
            legacy_id: w.id,
            code,
            title: w.title.clone(),
            circle_id,
            release_date,
            nsfw: w.nsfw.unwrap_or(false),
            dl_count: w.dl_count,
            price: w.price,
            rate_average: w.rate_average,
            rate_count: w.rate_count,
            metadata_src: if w.is_custom_meta == Some(1) {
                "manual"
            } else {
                "dlsite"
            },
            extra: Value::Object(extra).to_string(),
            created_at,
            updated_at: w.updated_at.unwrap_or(created_at),
            root_id,
            rel_path: w.dir.clone(),
            durations,
            cover: None,
        }
    }

    /// The `duration` object of the work's memo, with the durations from
    /// `t_track` on top. kikoeru 0.6.17 keeps them in `t_track` and drops
    /// them from the memo when it rescans a work, so either may be missing.
    fn durations(&mut self, w: &Work, tracks: Option<&Durations>, at: &str) -> Option<Value> {
        let memo = match w.memo.as_deref().map(serde_json::from_str::<Value>) {
            None | Some(Ok(Value::Null)) => None,
            Some(Ok(Value::Object(mut m))) => m.remove("duration").filter(Value::is_object),
            Some(_) => {
                self.report.issue(Issue::new(
                    at,
                    "memo is not a JSON object; durations dropped",
                ));
                None
            }
        };
        let Some(tracks) = tracks.filter(|t| !t.is_empty()) else {
            return memo;
        };
        self.report.preflight.track_durations += tracks.len() as u64;
        let mut merged = match memo {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        merged.extend(tracks.iter().map(|(k, v)| (k.clone(), v.clone())));
        Some(Value::Object(merged))
    }

    fn time(&mut self, ms: Option<i64>, at: &str, column: &str) -> i64 {
        ms.unwrap_or_else(|| {
            self.report.issue(Issue::new(
                at,
                format!("{column} is not a valid timestamp; using the import time"),
            ));
            self.now
        })
    }

    fn relations(&mut self, legacy: &Legacy, works: &HashMap<i64, usize>) {
        let tags: BTreeSet<i64> = legacy.tags.iter().map(|t| t.0).collect();
        let vas: BTreeSet<&str> = legacy.vas.iter().map(|v| v.0.as_str()).collect();
        self.report
            .count_source("r_tag_work", len(&legacy.work_tags));
        for (work_id, tag_id) in &legacy.work_tags {
            match works.get(work_id) {
                Some(&w) if tags.contains(tag_id) => {
                    self.plan.work_tags.push((w, *tag_id));
                    self.report.import("r_tag_work");
                }
                _ => self.report.skip(
                    "r_tag_work",
                    Issue::new(
                        format!("r_tag_work work_id={work_id} tag_id={tag_id}"),
                        "refers to a work or tag that is not imported",
                    ),
                ),
            }
        }
        self.report.count_source("r_va_work", len(&legacy.work_vas));
        for (work_id, va_id) in &legacy.work_vas {
            match works.get(work_id) {
                Some(&w) if vas.contains(va_id.as_str()) => {
                    self.plan.work_vas.push((w, va_id.clone()));
                    self.report.import("r_va_work");
                }
                _ => self.report.skip(
                    "r_va_work",
                    Issue::new(
                        format!("r_va_work work_id={work_id} va_id={va_id}"),
                        "refers to a work or voice actor that is not imported",
                    ),
                ),
            }
        }
    }

    fn users(&mut self, legacy: &Legacy) -> HashMap<String, usize> {
        self.report.count_source("t_user", len(&legacy.users));
        let mut index = HashMap::new();
        for u in &legacy.users {
            index.insert(u.name.clone(), self.plan.users.len());
            self.plan.users.push(UserRow {
                name: u.name.clone(),
                is_admin: u.group == "administrator",
            });
            self.report.import("t_user");
        }
        index
    }

    fn user_state(
        &mut self,
        legacy: &Legacy,
        works: &HashMap<i64, usize>,
        users: &HashMap<String, usize>,
    ) {
        let resolve =
            |user_name: &str, work_id: i64| Some((*users.get(user_name)?, *works.get(&work_id)?));
        // One play_progress row per (user, work), merged from both tables.
        let mut progress = BTreeMap::<(usize, usize), ProgressRow>::new();

        let mut reviewed = BTreeSet::new();
        self.report.count_source("t_review", len(&legacy.reviews));
        for r in &legacy.reviews {
            let at = format!("t_review user={} work_id={}", r.user_name, r.work_id);
            let Some((user, work)) = resolve(&r.user_name, r.work_id) else {
                self.report
                    .skip("t_review", Issue::new(at, "user or work is not imported"));
                continue;
            };
            reviewed.insert((user, work));
            self.review(r, user, work, &at, &mut progress);
        }

        self.report
            .count_source("t_play_histroy", len(&legacy.histories));
        for h in &legacy.histories {
            let at = format!("t_play_histroy user={} work_id={}", h.user_name, h.work_id);
            let Some((user, work)) = resolve(&h.user_name, h.work_id) else {
                self.report.skip(
                    "t_play_histroy",
                    Issue::new(at, "user or work is not imported"),
                );
                continue;
            };
            if reviewed.contains(&(user, work)) {
                self.report.preflight.review_history_overlap += 1;
            }
            self.history(h, user, work, &at, &mut progress);
        }
        self.plan.progress = progress.into_values().collect();
    }

    fn review(
        &mut self,
        r: &Review,
        user: usize,
        work: usize,
        at: &str,
        progress: &mut BTreeMap<(usize, usize), ProgressRow>,
    ) {
        let updated_at = r.updated_at.or(r.created_at);
        match r.progress.as_deref() {
            Some("marked") => {
                let at_time = self.time(updated_at, at, "updated_at");
                self.plan.favorites.push((user, work, at_time));
            }
            Some("listened") => {
                let finished_at = self.time(updated_at, at, "updated_at");
                progress
                    .entry((user, work))
                    .or_insert_with(|| ProgressRow {
                        user,
                        work,
                        position_ms: 0,
                        finished_at: None,
                        updated_at: finished_at,
                    })
                    .finished_at = Some(finished_at);
            }
            _ => {}
        }
        *self
            .report
            .preflight
            .review_progress
            .entry(r.progress.clone().unwrap_or_else(|| "null".to_owned()))
            .or_default() += 1;
        let carried = neo_core::review::from_legacy(r.rating, r.review_text.as_deref());
        if carried.dropped_rating {
            self.report.issue(Issue::new(
                at,
                "rating is not an integer from 1 to 5, so it is not shown",
            ));
        }
        if carried.truncated {
            self.report.issue(Issue::new(
                at,
                format!(
                    "review text is longer than {} characters and is shown cut",
                    neo_core::review::MAX_TEXT_CHARS
                ),
            ));
        }
        self.plan.reviews.push(ReviewRow {
            user,
            work,
            rating: r.rating,
            review_text: r.review_text.clone(),
            progress: r.progress.clone(),
            updated_at,
        });
        self.report.import("t_review");
    }

    fn history(
        &mut self,
        h: &History,
        user: usize,
        work: usize,
        at: &str,
        progress: &mut BTreeMap<(usize, usize), ProgressRow>,
    ) {
        let updated_at = self.time(h.updated_at, at, "updated_at");
        let parsed = parse_state(&h.state);
        let legacy_row = json!({ "user_name": h.user_name, "work_id": h.work_id });
        let (position_ms, source, state) = match parsed {
            Ok(mut p) => {
                if self.backslash {
                    p.use_slashes();
                }
                self.report.preflight.histories_valid += 1;
                let durations = self.plan.works[work].durations.as_ref();
                if durations.is_some_and(|d| d.get(&p.path).is_some()) {
                    self.report.preflight.queue_paths_in_memo += 1;
                }
                let source = json!({
                    "legacy_row": legacy_row,
                    "item": p.item,
                    "seconds": p.seconds,
                });
                (p.position_ms, source, STATE_PENDING)
            }
            Err(reason) => {
                self.report.issue(Issue::new(at, reason.message()));
                let source = json!({
                    "legacy_row": legacy_row,
                    "raw": h.state,
                    "reason": reason.message(),
                });
                (reason.position_ms(), source, STATE_UNMATCHED)
            }
        };
        if progress.contains_key(&(user, work)) {
            self.report.merge("t_play_histroy");
        } else {
            self.report.import("t_play_histroy");
        }
        let row = progress.entry((user, work)).or_insert_with(|| ProgressRow {
            user,
            work,
            position_ms: 0,
            finished_at: None,
            updated_at,
        });
        // The history supplies the position and its time. A finished mark
        // from a review stays as it is.
        row.position_ms = position_ms;
        row.updated_at = updated_at;
        self.plan.pending.push(PendingRow {
            user,
            work,
            source: source.to_string(),
            durations: self.plan.works[work]
                .durations
                .as_ref()
                .map(Value::to_string),
            state,
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedState {
    pub item: Value,
    /// `subtitle/title`: the file's path relative to the work directory.
    pub path: String,
    pub seconds: f64,
    pub position_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BadState {
    NotJson,
    NoQueue { position_ms: i64 },
    IndexOutOfRange { position_ms: i64 },
    NoTitle { position_ms: i64 },
}

impl BadState {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotJson => "state is not a JSON object",
            Self::NoQueue { .. } => "state has no queue array or integer index",
            Self::IndexOutOfRange { .. } => "index is outside the queue",
            Self::NoTitle { .. } => "queue item has no title",
        }
    }

    fn position_ms(&self) -> i64 {
        match self {
            Self::NotJson => 0,
            Self::NoQueue { position_ms }
            | Self::IndexOutOfRange { position_ms }
            | Self::NoTitle { position_ms } => *position_ms,
        }
    }
}

impl ParsedState {
    /// Turns `\\` in the queue item's folder into `/`, so the item matches
    /// the converted work directory and the scanner's paths.
    fn use_slashes(&mut self) {
        if let Some(Value::String(dir)) = self.item.get_mut("subtitle") {
            *dir = dir.replace('\\', "/");
        }
        self.path = self.path.replace('\\', "/");
    }
}

/// Parses an old `{queue, index, seconds}` play state. A missing, negative or
/// non-numeric `seconds` becomes position 0 rather than a failure, because
/// the queue item alone is still worth matching.
pub fn parse_state(state: &str) -> Result<ParsedState, BadState> {
    let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(state) else {
        return Err(BadState::NotJson);
    };
    let seconds = obj
        .get("seconds")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s >= 0.0)
        .unwrap_or(0.0);
    // Seconds of an audio file are far below 2^53 ms, so the cast is exact.
    let position_ms = (seconds * 1000.0).round() as i64;
    let index = obj.get("index").and_then(Value::as_u64);
    let (Some(Value::Array(mut queue)), Some(index)) = (obj.remove("queue"), index) else {
        return Err(BadState::NoQueue { position_ms });
    };
    let Some(item) = usize::try_from(index)
        .ok()
        .filter(|i| *i < queue.len())
        .map(|i| queue.swap_remove(i))
    else {
        return Err(BadState::IndexOutOfRange { position_ms });
    };
    let Some(title) = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return Err(BadState::NoTitle { position_ms });
    };
    let path = match item.get("subtitle").and_then(Value::as_str) {
        Some(dir) if !dir.is_empty() => format!("{dir}/{title}"),
        _ => title.to_owned(),
    };
    Ok(ParsedState {
        item,
        path,
        seconds,
        position_ms,
    })
}

/// Locations are joined onto a root directory later, so a path that is
/// absolute or climbs out with `..` would point outside the library. The
/// host's own parser also has to agree: on Windows `..\\x`, `C:x`, `\\x` and
/// `\\\\srv\\x` escape a root although they contain no `/`.
fn is_relative_inside(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path.split('/').all(|part| part != ".." && part != ".")
        && std::path::Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Whether the old library was written on Windows: some work directory
/// uses `\\` and none uses `/`. On Linux and macOS a name may contain `\\`
/// legally, so one `/` anywhere keeps every `\\` as it is.
fn windows_dirs(legacy: &Legacy) -> bool {
    windows_dirs_in(&legacy.works)
}

pub(crate) fn windows_dirs_in(works: &[Work]) -> bool {
    works.iter().any(|w| w.dir.contains('\\')) && !works.iter().any(|w| w.dir.contains('/'))
}

/// The old server gives circles it creates by hand IDs 0, -1, -2 and so
/// on, below every DLsite circle. API IDs must be positive, so these move to
/// a range above the old RG/BG/VG encoding (type * 10^9 + number), where no
/// DLsite circle ID can land.
const CUSTOM_CIRCLE_BASE: i64 = 9_000_000_000;

fn circle_id(old: i64) -> i64 {
    if old > 0 {
        old
    } else {
        CUSTOM_CIRCLE_BASE - old
    }
}

fn len<T>(v: &[T]) -> u64 {
    v.len() as u64
}

fn quote_list<'a>(items: impl IntoIterator<Item = &'a String>) -> String {
    items
        .into_iter()
        .map(|s| format!("{s:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::legacy::User;

    #[test]
    fn codes_from_legacy_ids() {
        let code = |id| Code::from_legacy_id(id).map(|c| c.to_string());
        assert_eq!(code(123_456).as_deref(), Some("RJ123456"));
        assert_eq!(code(1_234).as_deref(), Some("RJ001234"));
        assert_eq!(code(1_000_000).as_deref(), Some("RJ01000000"));
        assert_eq!(code(12_345_678).as_deref(), Some("RJ12345678"));
        assert_eq!(code(1_000_000_123_456).as_deref(), Some("BJ123456"));
        assert_eq!(code(2_000_001_246_802).as_deref(), Some("VJ01246802"));
        assert_eq!(code(3_000_000_000_001).as_deref(), Some("CC000001"));
        assert_eq!(code(4_000_000_000_001), None);
        assert_eq!(code(1_000_000_000_000), None);
        assert_eq!(code(0), None);
        assert_eq!(code(-5), None);
    }

    #[test]
    fn codes_in_dirs() {
        assert_eq!(codes_in_path("RJ123456"), vec![("RJ".into(), 123_456)]);
        assert_eq!(
            codes_in_path("a/rj0123456-x/VJ01246802"),
            vec![("RJ".into(), 123_456), ("VJ".into(), 1_246_802)]
        );
        assert_eq!(codes_in_path("音声RJ/RJ01"), vec![("RJ".into(), 1)]);
        assert!(codes_in_path("plain folder").is_empty());
        assert!(codes_in_path("RJ").is_empty());
    }

    #[test]
    fn play_states() {
        let ok = parse_state(
            r#"{"queue":[{"title":"a.mp3","subtitle":""},{"title":"b.mp3","subtitle":"mp3/SE"}],"index":1,"seconds":12.3456}"#,
        )
        .unwrap();
        assert_eq!(ok.path, "mp3/SE/b.mp3");
        assert_eq!(ok.position_ms, 12_346);
        assert_eq!(ok.item["title"], "b.mp3");

        let root = parse_state(r#"{"queue":[{"title":"a.mp3"}],"index":0,"seconds":-1}"#).unwrap();
        assert_eq!(root.path, "a.mp3");
        assert_eq!(root.position_ms, 0);

        assert_eq!(parse_state("{not json"), Err(BadState::NotJson));
        assert_eq!(parse_state("[]"), Err(BadState::NotJson));
        assert_eq!(
            parse_state(r#"{"queue":[],"seconds":1}"#),
            Err(BadState::NoQueue { position_ms: 1000 })
        );
        assert_eq!(
            parse_state(r#"{"queue":[{"title":"a"}],"index":1,"seconds":2}"#),
            Err(BadState::IndexOutOfRange { position_ms: 2000 })
        );
        assert_eq!(
            parse_state(r#"{"queue":[{"subtitle":"x"}],"index":0,"seconds":2}"#),
            Err(BadState::NoTitle { position_ms: 2000 })
        );
    }

    fn work(id: i64, root: &str, dir: &str) -> Work {
        Work {
            id,
            root_folder: root.into(),
            dir: dir.into(),
            title: "t".into(),
            circle_id: 1,
            nsfw: None,
            release: None,
            dl_count: None,
            price: None,
            rate_count: None,
            rate_average: None,
            rate_count_detail: None,
            rank: None,
            memo: None,
            is_custom_meta: None,
            created_at: Some(1),
            updated_at: None,
        }
    }

    #[test]
    fn refuses_works_that_would_share_a_location() {
        let legacy = Legacy {
            works: vec![
                work(100_001, "A", "RJ100001"),
                work(100_002, "B", "RJ100001"),
            ],
            ..Legacy::default()
        };
        let roots = BTreeMap::from([("A".into(), "x".into()), ("B".into(), "x".into())]);
        let (_, report, outcome) = build(&legacy, Some(&roots), 0);
        let Err(Refusal(lines)) = outcome else {
            panic!("expected a refusal");
        };
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("share the directory"), "{lines:?}");
        assert_eq!(report.preflight.duplicate_locations, 1);
        assert_eq!(report.preflight.dir_code_differs, 1);

        let separate = BTreeMap::from([("A".into(), "x".into()), ("B".into(), "y".into())]);
        assert!(build(&legacy, Some(&separate), 0).2.is_ok());
        assert!(build(&legacy, None, 0).2.is_ok());
    }

    #[test]
    fn locations_stay_inside_the_root() {
        assert!(is_relative_inside("RJ123456"));
        assert!(is_relative_inside("a/b/RJ123456 x"));
        for bad in [
            "",
            "/abs/RJ1",
            "../RJ1",
            "a/../../RJ1",
            "./RJ1",
            "a//RJ1/.",
            "a\0b",
        ] {
            assert!(!is_relative_inside(bad), "{bad:?}");
        }
        let legacy = Legacy {
            works: vec![work(100_001, "A", "../RJ100001")],
            ..Legacy::default()
        };
        let (plan, report, outcome) = build(&legacy, None, 0);
        assert!(outcome.is_ok());
        assert!(plan.works.is_empty());
        assert_eq!(report.tables["t_work"].skipped, 1);
    }

    #[test]
    fn windows_paths_cannot_leave_the_root_on_windows() {
        for bad in [r"..\x", r"C:x", r"C:\x", r"\\srv\x", r"\x"] {
            let inside = is_relative_inside(bad);
            // On Unix these are ordinary names, because `\` and `:` are
            // legal in file names there.
            assert_eq!(inside, !cfg!(windows), "{bad:?}");
        }
    }

    #[test]
    fn a_windows_library_gets_slashes_in_dirs_memo_and_queue() {
        let mut w = work(100_001, "A", r"voice\RJ100001");
        w.memo = Some(r#"{"duration":{"mp3\\01.mp3":60}}"#.into());
        let legacy = Legacy {
            works: vec![w],
            users: vec![User {
                name: "u".into(),
                group: "user".into(),
            }],
            histories: vec![History {
                user_name: "u".into(),
                work_id: 100_001,
                state: r#"{"queue":[{"title":"01.mp3","subtitle":"mp3"},{"title":"02.mp3","subtitle":"mp3\\b"}],"index":1,"seconds":1}"#.into(),
                updated_at: Some(1),
            }],
            ..Legacy::default()
        };
        let (plan, report, _) = build(&legacy, None, 0);
        assert!(report.preflight.backslash_dirs);
        assert_eq!(plan.works[0].rel_path, "voice/RJ100001");
        assert_eq!(plan.works[0].durations, Some(json!({"mp3/01.mp3": 60})));
        // t_track still wins over the memo for the same converted path.
        let mut with_track = legacy.clone();
        with_track.track_durations = BTreeMap::from([(
            100_001,
            Map::from_iter([(r"mp3\01.mp3".to_owned(), json!(61.5))]),
        )]);
        let (plan, _, _) = build(&with_track, None, 0);
        assert_eq!(plan.works[0].durations, Some(json!({"mp3/01.mp3": 61.5})));
        assert!(
            plan.pending[0].source.contains(r#""subtitle":"mp3/b""#),
            "{}",
            plan.pending[0].source
        );

        // One `/` anywhere means a Unix library, where `\` is part of a name.
        let mut unix = legacy.clone();
        unix.works.push(work(100_002, "A", "a/RJ100002"));
        let (plan, report, _) = build(&unix, None, 0);
        assert!(!report.preflight.backslash_dirs);
        assert_eq!(plan.works[0].rel_path, r"voice\RJ100001");
    }

    #[test]
    fn nfc_and_nfd_spellings_are_one_location() {
        let legacy = Legacy {
            works: vec![
                work(100_001, "A", "ボイス/RJ100001"),
                // The same name decomposed, as macOS writes it.
                work(100_002, "A", "ホ\u{3099}イス/RJ100001"),
            ],
            ..Legacy::default()
        };
        let (_, report, _) = build(&legacy, None, 0);
        assert_eq!(report.preflight.duplicate_locations, 1);
    }

    #[test]
    fn custom_circles_get_positive_ids() {
        let mut w = work(100_001, "A", "RJ100001");
        w.circle_id = -1;
        let legacy = Legacy {
            circles: vec![
                (-1, "made by hand".into()),
                (0, "zero".into()),
                (5, "dlsite".into()),
            ],
            works: vec![w],
            ..Legacy::default()
        };
        let (plan, report, _) = build(&legacy, None, 0);
        let ids: Vec<i64> = plan.circles.iter().map(|c| c.0).collect();
        assert_eq!(ids, [9_000_000_001, 9_000_000_000, 5]);
        assert_eq!(plan.works[0].circle_id, Some(9_000_000_001));
        assert_eq!(report.tables["t_circle"].imported, 3);
    }

    #[test]
    fn release_dates_that_do_not_exist_are_dropped() {
        let release = |id: i64, date: &str| Work {
            release: Some(date.into()),
            ..work(id, "A", &format!("RJ{id}"))
        };
        let legacy = Legacy {
            works: vec![
                release(100_001, "2024-02-29"),
                release(100_002, "2024-02-30"),
                release(100_003, "2024/01/31"),
            ],
            ..Legacy::default()
        };
        let (plan, report, outcome) = build(&legacy, None, 0);
        assert!(outcome.is_ok());
        let dates: Vec<_> = plan
            .works
            .iter()
            .map(|w| w.release_date.as_deref())
            .collect();
        assert_eq!(dates, [Some("2024-02-29"), None, None]);
        let dropped: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.reason.starts_with("release "))
            .map(|i| i.reason.as_str())
            .collect();
        assert_eq!(dropped.len(), 2, "{dropped:?}");
    }
}
