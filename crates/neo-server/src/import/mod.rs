//! `kikoeru-neo import-legacy`: the one-way import from an old kikoeru data
//! directory described in book/src/migration.md. It only ever reads the
//! source, and it only writes into a target directory that was empty.

pub mod config_out;
pub mod error;
pub mod legacy;
pub mod legacy_config;
pub mod merge;
pub mod plan;
pub mod report;
pub mod session;
pub mod snapshot;
pub mod source;
mod write;

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use neo_db::{self as db, Db, Location};

use crate::{cli::ImportArgs, datadir::DataDir, private::PrivateFile};
use error::{ErrorCode, ImportError, refuse};
use report::{Issue, Report};
use session::{Options, Session};
use snapshot::Journal;
use source::Given;
use tokio_util::sync::CancellationToken;

pub const REPORT_FILE: &str = "import-report.json";

/// A dry run found that the import would refuse. The binary exits with 2
/// for it, so a script can tell "would refuse" from a failure to check.
fn would_refuse() -> anyhow::Error {
    ImportError::new(
        ErrorCode::WouldRefuse,
        "the dry run found reasons the import would refuse; see above",
    )
    .into()
}

/// `--json`: the report and any refusal, for the desktop shell and scripts.
fn print_json(
    out: &mut impl io::Write,
    report: &Report,
    refusal: Option<&plan::Refusal>,
) -> Result<()> {
    let value = serde_json::json!({
        "report": report,
        "refusal": refusal.map(|r| &r.0),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

/// Runs the dry run, or the import when `target` is set: the data
/// directory and the library database it uses. Writes the summary to `out`.
/// Cancelling `cancel` stops a running import, which then cleans up the
/// target like any failed import.
pub async fn run(
    args: &ImportArgs,
    target: Option<(&Path, &Location)>,
    cancel: &CancellationToken,
    out: &mut impl io::Write,
) -> Result<()> {
    let database = target.map(|(_, db)| db);
    let target = target.map(|(dir, _)| dir);
    let mut roots: Option<BTreeMap<String, String>> = if args.roots.is_empty() {
        None
    } else {
        Some(root_map(&args.roots)?)
    };
    if !args.dry_run && target.is_none() {
        bail!("an import needs --data-dir or $KIKOERU_DATA_DIR for the new data directory");
    }
    let map = legacy_config::PathMap::parse(&args.map_paths)?;
    let tmp = std::env::temp_dir();
    let options = Options {
        given: Given {
            db: args.db.as_deref(),
            covers: args.covers.as_deref(),
            config: args.config.as_deref(),
            map: Some(&map),
        },
        accept_guessed: args.accept_guessed,
        journal: if args.recover_journal {
            Journal::Recover
        } else {
            Journal::Refuse
        },
        tmp: &tmp,
    };
    let session = Session::open(&args.source, &options).await?;
    let old_roots = session
        .source
        .settings
        .as_ref()
        .map(|s| {
            let data_dir = session
                .source
                .config
                .as_ref()
                .and_then(|c| source::data_dir_of_config(&c.path));
            config_out::old_roots(s, data_dir.as_deref(), &map)
        })
        .unwrap_or_default();
    if args.merge {
        let roots = roots.unwrap_or_default();
        let target = target.zip(database);
        return merge_into(args, &session, &roots, old_roots, target, cancel, out).await;
    }
    // A fresh import builds a new SQLite library that a failure can simply
    // delete. Into PostgreSQL, the library is copied once it is complete.
    if !args.dry_run && database.is_some_and(|db| !matches!(db, Location::Sqlite(_))) {
        refuse!(
            ErrorCode::NoLibrary,
            "a new library is imported into SQLite; import it without a PostgreSQL database \
             configured, then move it with `kikoeru-neo db copy`"
        );
    }
    let target = match target {
        Some(t) if !args.dry_run => Some(check_target(&args.source, &session, t)?),
        _ => None,
    };
    let legacy = &session.legacy;
    if !args.dry_run && roots.is_none() {
        // A library without works has no root_folder to map.
        if !legacy.works.is_empty() {
            bail!(
                "give every old root_folder a new root ID with --root OLD=NEW; run --dry-run to list them"
            );
        }
        roots = Some(BTreeMap::new());
    }
    let now = db::now_ms();
    let (mut plan, mut report, outcome) = plan::build(legacy, roots.as_ref(), now);
    report.source = Some(session.source.clone());
    report.old_roots = checked_roots(old_roots, &session.legacy, &mut report);
    attach_covers(&mut plan, &mut report, session.covers());

    let Some(target) = target else {
        let refusal = outcome.err();
        if args.json {
            print_json(out, &report, refusal.as_ref())?;
        } else {
            write!(out, "{}", report.render())?;
            if let Some(refusal) = &refusal {
                writeln!(out, "\nthe import would refuse to run:")?;
                for line in &refusal.0 {
                    writeln!(out, "  {line}")?;
                }
                if roots.is_none() {
                    writeln!(out, "  (pass --root OLD=NEW to check the root mapping too)")?;
                }
            }
        }
        return refusal.map_or(Ok(()), |_| Err(would_refuse()));
    };
    if let Err(refusal) = outcome {
        refuse!(
            ErrorCode::WouldRefuse,
            "refusing to import:\n  {}",
            refusal.0.join("\n  ")
        );
    }

    let created = claim_target(&target)?;
    let result = import(&plan, &mut report, &target, now, cancel).await;
    if let Err(e) = result {
        // A partial import must not be mistaken for a data directory.
        return Err(match remove_output(&target, created) {
            Ok(()) => e.context(format!(
                "import failed; {} was cleaned up",
                target.display()
            )),
            Err(c) => e.context(format!(
                "import failed and {} could not be cleaned up ({c}); delete it before reusing it",
                target.display()
            )),
        });
    }
    fs::remove_file(target.join(LOCK_FILE))
        .with_context(|| format!("cannot unlock {}", target.display()))?;
    write!(out, "{}", report.render())?;
    write_config(
        args,
        roots.as_ref().unwrap_or(&BTreeMap::new()),
        &report,
        &target,
        out,
    )?;
    writeln!(
        out,
        "\nimported into {}; the full report with the work ID map is {}",
        target.display(),
        target.join(REPORT_FILE).display()
    )?;
    writeln!(
        out,
        "set a password before logging in: kikoeru-neo --data-dir {} user set-password <name>",
        target.display()
    )?;
    Ok(())
}

/// Device and inode on Unix, volume and file index on Windows.
fn same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// Spot-checks the old local libraries and reports a folder that holds
/// fewer than half of the sampled works.
fn checked_roots(
    mut old: Vec<config_out::OldRoot>,
    legacy: &legacy::Legacy,
    report: &mut Report,
) -> Vec<config_out::OldRoot> {
    config_out::spot_check(&mut old, &legacy.works);
    for r in &old {
        if let (Some(s), Some(p)) = (r.sample, &r.path)
            && s.is_low()
        {
            report.issue(Issue::new(
                format!("old library {}", r.name),
                format!(
                    "only {} of {} sampled works are in {}; check that this is the right folder",
                    s.found,
                    s.checked,
                    p.path.display()
                ),
            ));
        }
    }
    old
}

/// `--write-config`: the old media libraries as `[[roots]]` for the
/// library in `target`.
fn write_config(
    args: &ImportArgs,
    roots: &BTreeMap<String, String>,
    report: &Report,
    target: &Path,
    out: &mut impl io::Write,
) -> Result<()> {
    let Some(path) = &args.write_config else {
        return Ok(());
    };
    let text = config_out::render(roots, &report.old_roots, target);
    config_out::write(path, &text)?;
    writeln!(
        out,
        "wrote [[roots]] for the old media libraries to {}; check its TODO lines and merge it into config.toml",
        path.display()
    )?;
    Ok(())
}

/// `--merge`: the old user data goes into the library at `target`, which
/// must already exist. Nothing else in it changes.
async fn merge_into(
    args: &ImportArgs,
    session: &Session,
    roots: &BTreeMap<String, String>,
    old_roots: Vec<config_out::OldRoot>,
    target: Option<(&Path, &Location)>,
    cancel: &CancellationToken,
    out: &mut impl io::Write,
) -> Result<()> {
    let now = db::now_ms();
    let (plan, mut report, outcome) = plan::build(&session.legacy, None, now);
    report.source = Some(session.source.clone());
    report.old_roots = checked_roots(old_roots, &session.legacy, &mut report);
    let users = merge::user_map(&args.users, &plan)?;
    let with_data = merge::states_len(&plan);
    if args.dry_run {
        let refusal = outcome.err();
        if args.json {
            print_json(out, &report, refusal.as_ref())?;
        } else {
            write!(out, "{}", report.render())?;
            writeln!(out, "\nwould merge user data for {with_data} works")?;
            if let Some(refusal) = &refusal {
                writeln!(out, "\nthe merge would refuse to run:")?;
                for line in &refusal.0 {
                    writeln!(out, "  {line}")?;
                }
            }
        }
        return refusal.map_or(Ok(()), |_| Err(would_refuse()));
    }
    if let Err(refusal) = outcome {
        refuse!(
            ErrorCode::WouldRefuse,
            "refusing to merge:\n  {}",
            refusal.0.join("\n  ")
        );
    }
    let Some((target, database)) = target else {
        bail!("--merge needs --data-dir or $KIKOERU_DATA_DIR for the library to merge into");
    };
    // The library is opened for writing, so neither it nor a database file
    // it links to may be part of the source.
    let target = &check_target(&args.source, session, target)?;
    if let Ok(real) = fs::canonicalize(target.join("neo.sqlite3"))
        && let Some(dir) = real.parent()
    {
        check_target(&args.source, session, dir)?;
        // A hard link has no path to compare, only the same file identity.
        if same_file(&real, &session.source.db.path) {
            refuse!(
                ErrorCode::TargetOverlaps,
                "{} is the old database itself; merge into a library of its own",
                real.display()
            );
        }
    }
    // Nothing is created here, so a mistyped path is not mistaken for an
    // empty library to merge into.
    let data_dir = DataDir::existing(target).map_err(|e| {
        ImportError::new(
            ErrorCode::NoLibrary,
            format!("{} has no library to merge into: {e}", target.display()),
        )
    })?;
    // A PostgreSQL library is wherever its URL points, so only a SQLite
    // file can be found missing here.
    if let Location::Sqlite(path) = database
        && !path.is_file()
    {
        refuse!(
            ErrorCode::NoLibrary,
            "{} has no library to merge into; import without --merge to create one",
            data_dir.root().display()
        );
    }
    let db = Db::open(database).await?;
    let result = merge::merge(&db, &plan, &users, now, cancel).await;
    db.close().await;
    let merged = result?;
    // The plan's issues name the old rows that could not come over.
    write!(out, "{}", report.render())?;
    writeln!(out)?;
    write_config(args, roots, &report, target, out)?;
    let c = &merged.claimed;
    writeln!(out, "merged user data for {} works", merged.works)?;
    writeln!(
        out,
        "attached to works already in the library: {} favorites, {} reviews, {} positions \
         ({} positions kept because the library's were newer)",
        c.favorites, c.reviews, c.progress, c.progress_kept
    )?;
    writeln!(
        out,
        "works waiting for a scan: {}; their data appears once a scan finds them",
        merged.waiting
    )?;
    if !merged.users_created.is_empty() {
        writeln!(
            out,
            "created without a password: {}; set one with: kikoeru-neo --data-dir {} user set-password <name>",
            merged.users_created.join(", "),
            target.display()
        )?;
    }
    Ok(())
}

async fn import(
    plan: &plan::Plan,
    report: &mut Report,
    target: &Path,
    now: i64,
    cancel: &CancellationToken,
) -> Result<()> {
    let data_dir = DataDir::prepare(target)?;
    let db = Db::open_sqlite(&data_dir.database()).await?;
    let result = async {
        let ids = write::write(&db, plan, &data_dir.root().join("covers"), now, cancel).await?;
        report.work_ids = plan.works.iter().map(|w| w.legacy_id).zip(ids).collect();
        let expected: Vec<_> = report
            .written
            .iter()
            .filter(|(t, _)| !matches!(**t, "covers" | "play_progress_finished"))
            .map(|(t, n)| (*t, *n))
            .collect();
        write::verify(&db, plan, &expected).await
    }
    .await;
    db.close().await;
    result?;
    // The data directory's files are private, like the database.
    let path = target.join(REPORT_FILE);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(&path)
        .and_then(|mut f| serde_json::to_writer_pretty(&mut f, report).map_err(io::Error::from))
        .with_context(|| format!("cannot write {}", path.display()))
}

const LOCK_FILE: &str = ".import-lock";

/// Every name the import creates in the target: the layout from
/// `DataDir::prepare`, the database with its side files, and the report.
const OUTPUT_NAMES: [&str; 9] = [
    "neo.sqlite3",
    "neo.sqlite3-wal",
    "neo.sqlite3-shm",
    "neo.sqlite3-journal",
    "secrets",
    "covers",
    "lyrics",
    "cache",
    REPORT_FILE,
];

/// Removes only what the import created, so a file that appeared in the
/// target meanwhile is never deleted. A directory the import created is
/// removed too, if nothing else is left in it.
fn remove_output(target: &Path, created: bool) -> io::Result<()> {
    for name in OUTPUT_NAMES {
        let path = target.join(name);
        let removed = match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&path),
            Ok(_) => fs::remove_file(&path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        removed?;
    }
    // The lock goes last, so another import cannot take the directory while
    // this one is still removing its files.
    fs::remove_file(target.join(LOCK_FILE))?;
    if created {
        fs::remove_dir(target)?;
    }
    Ok(())
}

/// Creates the target, or takes over an existing empty directory. Returns
/// whether the import created it. Deciding this from the create call itself,
/// not from an earlier look, keeps a directory that appears in between from
/// being treated as ours.
fn claim_target(target: &Path) -> Result<bool> {
    let created = create_or_reuse(target)?;
    lock_target(target)?;
    // Another import may have finished in this directory between the
    // emptiness check and the lock, so only the lock may be there now.
    let others = fs::read_dir(target)?
        .filter_map(Result::ok)
        .any(|e| e.file_name() != LOCK_FILE);
    if others {
        fs::remove_file(target.join(LOCK_FILE))?;
        refuse!(
            ErrorCode::TargetNotEmpty,
            "target {} is not empty; import into a new directory",
            target.display()
        );
    }
    Ok(created)
}

fn create_or_reuse(target: &Path) -> Result<bool> {
    if let Some(parent) = target.parent() {
        crate::private::dir()
            .recursive(true)
            .create(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    match crate::private::dir().create(target) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let meta = fs::symlink_metadata(target)?;
            if !meta.is_dir() {
                refuse!(
                    ErrorCode::TargetNotEmpty,
                    "target {} exists and is not a directory",
                    target.display()
                );
            }
            if fs::read_dir(target)?.next().is_some() {
                refuse!(
                    ErrorCode::TargetNotEmpty,
                    "target {} is not empty; import into a new directory",
                    target.display()
                );
            }
            Ok(false)
        }
        Err(e) => Err(e).with_context(|| format!("cannot create {}", target.display())),
    }
}

/// Takes the target for this process. Two imports can both find the same
/// directory empty, but only one can create the lock file, and the other
/// stops before writing or cleaning up anything.
fn lock_target(target: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(target.join(LOCK_FILE))
        .map(drop)
        .with_context(|| {
            format!(
                "cannot lock {}; is another import writing to it?",
                target.display()
            )
        })
}

fn attach_covers(plan: &mut plan::Plan, report: &mut Report, dir: Option<&Path>) {
    let mut found = 0;
    for w in &mut plan.works {
        let Some(dir) = dir else {
            report.issue(Issue::new(
                format!("t_work id={}", w.legacy_id),
                "no covers folder",
            ));
            continue;
        };
        let path = dir.join(format!("{}_img_main.jpg", w.code));
        // A symlink in the copy could point anywhere, so only regular files
        // count as covers.
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            w.cover = Some(path);
            found += 1;
        } else {
            report.issue(Issue::new(
                format!("t_work id={}", w.legacy_id),
                format!("no cover {}_img_main.jpg", w.code),
            ));
        }
    }
    report.written.insert("covers", found);
}

/// Parses `--root OLD=NEW` pairs. New IDs follow the rule for `roots[].id`
/// in the config, because the config must name the same IDs later.
fn root_map(pairs: &[String]) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for pair in pairs {
        // A new ID never contains '=', but an old folder name may.
        let Some((old, new)) = pair.rsplit_once('=') else {
            bail!("--root {pair:?} is not OLD=NEW");
        };
        if !crate::config::is_valid_root_id(new) {
            bail!(
                "--root {pair:?}: the new ID must be non-empty and use only ASCII letters, digits, '-' and '_'"
            );
        }
        if map.insert(old.to_owned(), new.to_owned()).is_some() {
            bail!("--root names {old:?} twice");
        }
    }
    Ok(map)
}

/// The target must not overlap what the user chose or the folders the import
/// reads from, and must not be a dangling symlink. Returns it as an
/// absolute path with symlinks resolved. `claim_target` checks that it is
/// missing or empty.
fn check_target(pick: &Path, session: &Session, target: &Path) -> Result<PathBuf> {
    let target = absolute(target)?;
    // The real file, so a symlink or a bare relative name still yields the
    // folders the database actually lives in.
    let db = fs::canonicalize(&session.source.db.path)
        .with_context(|| format!("cannot open source {}", session.source.db.path.display()))?;
    // A file was picked for itself. Its folder is checked as the db's.
    let pick = (!pick.is_file()).then_some(pick);
    let db_dir = db.parent();
    // The data directory above sqlite/, which holds the old config too.
    let data_dir = db_dir
        .filter(|d| d.file_name().is_some_and(|n| n == "sqlite"))
        .and_then(Path::parent);
    for read in [pick, db_dir, data_dir, session.covers()]
        .into_iter()
        .flatten()
    {
        let read = fs::canonicalize(read)
            .with_context(|| format!("cannot open source {}", read.display()))?;
        if target.starts_with(&read) || read.starts_with(&target) {
            refuse!(
                ErrorCode::TargetOverlaps,
                "target {} overlaps the source {}",
                target.display(),
                read.display()
            );
        }
    }
    Ok(target)
}

/// Resolves symlinks in the part of `path` that exists, so that a target
/// reached through a link is still compared with the real source path.
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    // symlink_metadata sees a dangling link as existing, so canonicalize
    // below rejects it instead of the import creating a directory over it.
    while fs::symlink_metadata(existing).is_err() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            break;
        };
        rest.push(name.to_owned());
        existing = parent;
    }
    let mut resolved = fs::canonicalize(existing)
        .with_context(|| format!("cannot resolve {}", existing.display()))?;
    resolved.extend(rest.iter().rev());
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_import_claims_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new");
        assert!(claim_target(&target).unwrap());
        // A second import that saw the directory empty before the lock
        // existed still cannot take it.
        assert!(lock_target(&target).is_err());
        assert!(
            claim_target(&target).is_err(),
            "the lock makes it non-empty"
        );
        remove_output(&target, true).unwrap();
        assert!(!target.exists());

        // A finished import that appeared after the emptiness check stays.
        fs::create_dir(&target).unwrap();
        assert!(!create_or_reuse(&target).unwrap());
        fs::write(target.join("neo.sqlite3"), "other import").unwrap();
        assert!(claim_target(&target).is_err());
        let left: Vec<_> = fs::read_dir(&target)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["neo.sqlite3"]);
    }
}
