//! The preflight summary and import report from book/src/migration.md. The
//! dry run prints it, and the import also saves it with the old-to-new work
//! ID map as `import-report.json` in the new data directory.

use std::{collections::BTreeMap, fmt::Write as _};

use serde::Serialize;

use super::{
    config_out::OldRoot,
    plan::{Plan, STATE_PENDING, STATE_UNMATCHED},
    source::Source,
};

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct TableCount {
    pub source: u64,
    pub imported: u64,
    pub merged: u64,
    pub skipped: u64,
}

/// One row that was skipped, changed or could not be matched, with where it
/// came from in the old database.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Issue {
    pub at: String,
    pub reason: String,
}

impl Issue {
    pub fn new(at: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Preflight {
    /// Work count by `prefix/digits`, such as `RJ/8`.
    pub codes: BTreeMap<String, u64>,
    pub root_folders: BTreeMap<String, u64>,
    /// Work directories and queue folders used `\\` and were converted.
    pub backslash_dirs: bool,
    pub duplicate_locations: u64,
    pub unparsable_dirs: u64,
    pub dir_code_differs: u64,
    /// Directories that hold the right number with a different width.
    pub dir_code_nonstandard: u64,
    pub review_progress: BTreeMap<String, u64>,
    pub review_history_overlap: u64,
    pub histories_valid: u64,
    pub queue_paths_in_memo: u64,
    /// Track durations read from `t_track` rather than the memo.
    pub track_durations: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Where the old database and covers were found, and how.
    pub source: Option<Source>,
    /// The old config's media libraries, mapped onto this machine.
    pub old_roots: Vec<OldRoot>,
    pub last_migration: Option<String>,
    /// Per old table: rows read, imported, merged into another row, skipped.
    pub tables: BTreeMap<&'static str, TableCount>,
    /// Rows the new database gets, per new table.
    pub written: BTreeMap<&'static str, u64>,
    pub import_pending: BTreeMap<&'static str, u64>,
    pub preflight: Preflight,
    pub issues: Vec<Issue>,
    /// `[old ID, new ID]` pairs, filled in after the works are inserted.
    pub work_ids: Vec<(i64, i64)>,
}

impl Report {
    pub fn new(last_migration: Option<String>) -> Self {
        Self {
            source: None,
            old_roots: Vec::new(),
            last_migration,
            tables: BTreeMap::new(),
            written: BTreeMap::new(),
            import_pending: BTreeMap::new(),
            preflight: Preflight::default(),
            issues: Vec::new(),
            work_ids: Vec::new(),
        }
    }

    fn table(&mut self, table: &'static str) -> &mut TableCount {
        self.tables.entry(table).or_default()
    }

    pub fn count_source(&mut self, table: &'static str, n: u64) {
        self.table(table).source += n;
    }

    pub fn import(&mut self, table: &'static str) {
        self.import_n(table, 1);
    }

    pub fn import_n(&mut self, table: &'static str, n: u64) {
        self.table(table).imported += n;
    }

    pub fn merge(&mut self, table: &'static str) {
        self.table(table).merged += 1;
    }

    pub fn skip(&mut self, table: &'static str, issue: Issue) {
        self.skip_n(table, 1, Some(issue));
    }

    pub fn skip_n(&mut self, table: &'static str, n: u64, issue: Option<Issue>) {
        self.table(table).skipped += n;
        self.issues.extend(issue);
    }

    pub fn issue(&mut self, issue: Issue) {
        self.issues.push(issue);
    }

    pub fn set_written(&mut self, plan: &Plan) {
        let n = |len: usize| len as u64;
        let tags = n(plan.tags.len());
        self.written = BTreeMap::from([
            ("roots", n(plan.roots.len())),
            ("circles", n(plan.circles.len())),
            ("vas", n(plan.vas.len())),
            ("tags", tags),
            ("works", n(plan.works.len())),
            ("work_locations", n(plan.works.len())),
            ("work_tags", n(plan.work_tags.len())),
            ("work_vas", n(plan.work_vas.len())),
            ("users", n(plan.users.len())),
            ("favorites", n(plan.favorites.len())),
            ("play_progress", n(plan.progress.len())),
            ("legacy_reviews", n(plan.reviews.len())),
            ("import_pending", n(plan.pending.len())),
        ]);
        // Each tag's old name becomes its alias. Two tags with the same name
        // share one alias, which points at the first of them.
        let mut names: Vec<&str> = plan.tags.iter().map(|t| t.1.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        self.written.insert("tag_aliases", n(names.len()));
        self.written.insert(
            "play_progress_finished",
            n(plan
                .progress
                .iter()
                .filter(|p| p.finished_at.is_some())
                .count()),
        );
        for state in [STATE_PENDING, STATE_UNMATCHED] {
            self.import_pending.insert(
                state,
                n(plan.pending.iter().filter(|p| p.state == state).count()),
            );
        }
    }

    /// The summary printed to stdout. It leaves out the ID map, which only
    /// goes into the saved JSON.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let p = &self.preflight;
        if let Some(source) = &self.source {
            let _ = writeln!(
                out,
                "old database: {} ({})",
                source.db.path.display(),
                source.db.origin
            );
            match &source.covers {
                Some(c) => {
                    let _ = writeln!(out, "old covers: {} ({})", c.path.display(), c.origin);
                }
                None => {
                    let _ = writeln!(out, "old covers: none found");
                }
            }
            if let Some(c) = &source.config {
                let version = source
                    .settings
                    .as_ref()
                    .and_then(|s| s.version.as_deref())
                    .unwrap_or("unknown version");
                let _ = writeln!(
                    out,
                    "old config: {} ({}, {version})",
                    c.path.display(),
                    c.origin
                );
            }
        }
        let _ = writeln!(
            out,
            "last old migration: {}",
            self.last_migration.as_deref().unwrap_or("none")
        );
        let _ = writeln!(
            out,
            "\nold table          source  imported  merged  skipped"
        );
        for (table, c) in &self.tables {
            let _ = writeln!(
                out,
                "{table:<17} {:>7} {:>9} {:>7} {:>8}",
                c.source, c.imported, c.merged, c.skipped
            );
        }
        let _ = writeln!(out, "\nnew table                 rows");
        for (table, n) in &self.written {
            let _ = writeln!(out, "{table:<24} {n:>5}");
        }
        for (state, n) in &self.import_pending {
            let _ = writeln!(out, "import_pending {state:<10} {n:>5}");
        }
        let _ = writeln!(out, "\npreflight");
        let _ = writeln!(out, "  codes by prefix/digits: {}", pairs(&p.codes));
        let _ = writeln!(out, "  root_folder: {}", pairs(&p.root_folders));
        if p.backslash_dirs {
            let _ = writeln!(out, "  folders written on Windows: every \\ became /");
        }
        for r in &self.old_roots {
            let place = match (&r.path, &r.old_path, &r.url) {
                (Some(p), _, _) => format!("{} ({})", p.path.display(), p.origin),
                (None, Some(old), _) => format!("{old} (not found on this machine)"),
                (None, None, Some(url)) => {
                    format!("{url}{}", r.base_path.as_deref().unwrap_or_default())
                }
                _ => "no path".to_owned(),
            };
            let sample = r.sample.map_or(String::new(), |s| {
                format!(", {} of {} sampled works found", s.found, s.checked)
            });
            let _ = writeln!(out, "  old library {}: {} {place}{sample}", r.name, r.kind);
        }
        let _ = writeln!(out, "  review progress: {}", pairs(&p.review_progress));
        for (label, n) in [
            ("duplicate locations", p.duplicate_locations),
            ("directories without a code", p.unparsable_dirs),
            ("directories with another code", p.dir_code_differs),
            (
                "directories with a nonstandard code width",
                p.dir_code_nonstandard,
            ),
            ("reviewed works with play history", p.review_history_overlap),
            ("valid play states", p.histories_valid),
            ("queue paths found in memo", p.queue_paths_in_memo),
            ("track durations from t_track", p.track_durations),
        ] {
            let _ = writeln!(out, "  {label}: {n}");
        }
        let _ = writeln!(out, "\nissues: {}", self.issues.len());
        for issue in &self.issues {
            let _ = writeln!(out, "  {}: {}", issue.at, issue.reason);
        }
        out
    }
}

fn pairs<K: std::fmt::Display>(map: &BTreeMap<K, u64>) -> String {
    if map.is_empty() {
        return "none".to_owned();
    }
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}
