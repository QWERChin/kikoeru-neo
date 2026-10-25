//! `kikoeru-neo migrate`: the guided import from an old kikoeru install. It
//! asks in Chinese for what `import-legacy` takes as flags, then runs it,
//! writes the media libraries into the config and sets passwords. It works
//! the same on a bare host, under NixOS and inside the Docker image, which
//! has no shell tools. book/src/migration.md, "引导式迁移", describes it.

use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use neo_db::{self as db, Db, users};
use tokio_util::sync::CancellationToken;

use crate::{
    cli::{ImportArgs, MigrateArgs},
    config::is_valid_root_id,
    datadir::DataDir,
    import::{
        self,
        config_out::{self, OldRoot},
        legacy::Work,
        legacy_config::PathMap,
        plan,
        session::{Options, Session},
        snapshot::Journal,
        source::{self, Given, Located, Origin},
    },
    password, user_cmd,
};

/// Questions and answers. The terminal answers them in a real run, and
/// tests script them.
pub trait Prompt {
    fn say(&mut self, text: &str) -> Result<()>;
    /// A line of text. An empty answer takes `default`.
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String>;
    fn confirm(&mut self, question: &str, default: bool) -> Result<bool>;
    /// A password, or `None` to skip. `new_account` applies the length rule
    /// for this server's accounts. An existing OpenList password is taken
    /// as it is.
    fn password(&mut self, question: &str, new_account: bool) -> Result<Option<String>>;
}

/// The terminal. Passwords are read without echo.
pub struct Terminal;

impl Prompt for Terminal {
    fn say(&mut self, text: &str) -> Result<()> {
        println!("{text}");
        Ok(())
    }

    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String> {
        match default {
            Some(d) => print!("{question} [{d}]: "),
            None => print!("{question}: "),
        }
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            bail!("standard input closed");
        }
        let line = line.trim();
        Ok(match (line.is_empty(), default) {
            (true, Some(d)) => d.to_owned(),
            _ => line.to_owned(),
        })
    }

    fn confirm(&mut self, question: &str, default: bool) -> Result<bool> {
        let hint = if default { "Y/n" } else { "y/N" };
        loop {
            let answer = self.ask(&format!("{question} ({hint})"), Some(""))?;
            match answer.to_lowercase().as_str() {
                "" => return Ok(default),
                "y" | "yes" | "是" => return Ok(true),
                "n" | "no" | "否" => return Ok(false),
                _ => println!("请回答 y 或 n"),
            }
        }
    }

    fn password(&mut self, question: &str, new_account: bool) -> Result<Option<String>> {
        loop {
            let first = rpassword::prompt_password(format!("{question}（直接回车跳过）: "))?;
            if first.is_empty() {
                return Ok(None);
            }
            if new_account && !password::is_long_enough(&first) {
                println!("密码太短，请重新输入");
                continue;
            }
            let second = rpassword::prompt_password("再输入一次: ")?;
            if first == second {
                return Ok(Some(first));
            }
            println!("两次输入不一致，请重新输入");
        }
    }
}

/// `--yes` without a terminal: every question takes its default, and no
/// password is set.
pub struct Defaults;

impl Prompt for Defaults {
    fn say(&mut self, text: &str) -> Result<()> {
        println!("{text}");
        Ok(())
    }

    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String> {
        default
            .map(str::to_owned)
            .with_context(|| format!("{question}: no default answer; pass it as a flag"))
    }

    fn confirm(&mut self, _: &str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn password(&mut self, _: &str, _: bool) -> Result<Option<String>> {
        Ok(None)
    }
}

pub async fn run(
    args: &MigrateArgs,
    data_dir: &Path,
    cancel: &CancellationToken,
    prompt: &mut impl Prompt,
    out: &mut impl Write,
) -> Result<()> {
    // Paths written into the config must not depend on where the server
    // is started from.
    let data_dir = &std::path::absolute(data_dir)?;
    let source = match &args.source {
        Some(s) => s.clone(),
        None => PathBuf::from(prompt.ask(
            "旧数据在哪里？（旧安装目录、它的 sqlite 或 config 文件夹、db.sqlite3，或 Docker 卷目录）",
            None,
        )?),
    };
    let map = PathMap::parse(&args.map_paths)?;
    let given = Given {
        map: Some(&map),
        ..Given::default()
    };
    let found = source::locate(&source, &given)?;
    prompt.say(&format!(
        "旧数据库：{}（{}）",
        found.db.path.display(),
        found.db.origin
    ))?;
    match &found.covers {
        Some(c) => prompt.say(&format!("旧封面：{}（{}）", c.path.display(), c.origin))?,
        None => prompt.say("没有找到旧封面目录，导入后可以重新获取封面")?,
    }
    let guessed = !found.guessed().is_empty();
    if guessed && !prompt.confirm("上面有推测出来的路径，确认是你要导入的那份数据吗？", false)?
    {
        bail!("没有确认推测的路径；可以直接指定旧安装目录再试一次");
    }
    let recover = if has_journal(&found.db.path) {
        prompt.say("旧数据库旁边有未合并的日志文件：旧服务可能还在运行，或者上次没有正常退出。")?;
        if !prompt.confirm(
            "已经停掉旧服务了吗？确认后在副本上恢复，旧文件不会被改动",
            false,
        )? {
            bail!("先停掉旧服务再运行 migrate");
        }
        true
    } else {
        false
    };

    let tmp = std::env::temp_dir();
    let options = Options {
        given: given.clone(),
        accept_guessed: guessed,
        journal: if recover {
            Journal::Recover
        } else {
            Journal::Refuse
        },
        tmp: &tmp,
    };
    let session = Session::open(&source, &options).await?;
    let merge = DataDir::existing(data_dir).is_ok_and(|d| d.database().is_file());
    if merge {
        prompt.say(&format!(
            "{} 已经有一个库。只合并旧的收藏、review 和播放进度，作品由这个库自己扫描。",
            data_dir.display()
        ))?;
    }

    let (_, report, _) = plan::build(&session.legacy, None, db::now_ms());
    let preset = preset_roots(&args.roots)?;
    let mut old_roots = session
        .source
        .settings
        .as_ref()
        .map(|s| {
            let dir = session
                .source
                .config
                .as_ref()
                .and_then(|c| source::data_dir_of_config(&c.path));
            config_out::old_roots(s, dir.as_deref(), &map)
        })
        .unwrap_or_default();
    config_out::spot_check(&mut old_roots, &session.legacy.works);
    let mut roots = BTreeMap::new();
    // Libraries named only in the old config have no works yet, but still
    // belong in the new config.
    let mut names = report.preflight.root_folders.clone();
    for r in &old_roots {
        names.entry(r.name.clone()).or_insert(0);
    }
    for (name, works) in &names {
        let default = preset
            .get(name)
            .cloned()
            .unwrap_or_else(|| suggest_id(name, &roots));
        prompt.say(&format!("旧媒体库 {name}：{works} 部作品"))?;
        let id = loop {
            let id = prompt.ask("  新的媒体库 ID（英文字母、数字、- 和 _）", Some(&default))?;
            if is_valid_root_id(&id) {
                break id;
            }
            prompt.say("  ID 只能用英文字母、数字、- 和 _")?;
        };
        roots.insert(name.clone(), id);
        ask_path(prompt, name, &mut old_roots, &session.legacy.works)?;
    }

    let config_path = config_target(data_dir);
    let import_args = ImportArgs {
        source: source.clone(),
        db: None,
        covers: None,
        config: None,
        map_paths: args.map_paths.clone(),
        accept_guessed: guessed,
        recover_journal: recover,
        dry_run: true,
        json: false,
        // import-legacy refuses a --root that no work uses.
        roots: roots
            .iter()
            .filter(|(k, _)| report.preflight.root_folders.contains_key(*k))
            .map(|(k, v)| format!("{k}={v}"))
            .collect(),
        write_config: None,
        merge,
        users: Vec::new(),
    };
    // `main` refuses a PostgreSQL library before getting here.
    let sqlite = neo_db::Location::Sqlite(data_dir.join(crate::datadir::DATABASE_FILE));
    import::run(&import_args, Some((data_dir, &sqlite)), cancel, out).await?;
    if !prompt.confirm("按上面的预检结果开始导入？", true)? {
        bail!("没有导入");
    }
    let import_args = ImportArgs {
        dry_run: false,
        ..import_args
    };
    import::run(&import_args, Some((data_dir, &sqlite)), cancel, out).await?;
    openlist_passwords(prompt, &roots, &old_roots, data_dir)?;
    config_out::write(
        &config_path,
        &config_out::render(&roots, &old_roots, data_dir),
    )?;
    prompt.say(&format!(
        "旧媒体库写进了 {}，请检查其中的 TODO 行",
        config_path.display()
    ))?;

    set_passwords(data_dir, prompt).await
}

/// Asks where a local media library is now, when the old config did not
/// say or its folder was not found. An empty answer leaves a TODO in the
/// written config.
fn ask_path(
    prompt: &mut impl Prompt,
    name: &str,
    old: &mut Vec<OldRoot>,
    works: &[Work],
) -> Result<()> {
    let index = match old.iter().position(|r| r.name == name) {
        Some(i) => i,
        None => {
            old.push(OldRoot {
                name: name.to_owned(),
                kind: "local".to_owned(),
                old_path: None,
                path: None,
                url: None,
                base_path: None,
                username: None,
                sample: None,
            });
            old.len() - 1
        }
    };
    let root = &mut old[index];
    if root.kind != "local" {
        return Ok(());
    }
    if let Some(p) = &root.path {
        if p.origin != Origin::Guessed {
            prompt.say(&format!(
                "  现在的位置：{}（{}）",
                p.path.display(),
                p.origin
            ))?;
            let Some(s) = root.sample.filter(config_out::Sample::is_low) else {
                return Ok(());
            };
            let question = format!(
                "  抽查的 {} 部作品只在这里找到 {} 部，可能不是这个目录，确认用它吗？",
                s.checked, s.found
            );
            if prompt.confirm(&question, false)? {
                return Ok(());
            }
            root.path = None;
            root.sample = None;
            return ask_folder(prompt, root, works);
        }
        let question = format!("  推测它现在在 {}，对吗？", p.path.display());
        if prompt.confirm(&question, false)? {
            root.path = root.path.take().map(|p| Located {
                origin: Origin::Given,
                ..p
            });
            return Ok(());
        }
        root.path = None;
    }
    ask_folder(prompt, root, works)
}

/// The old config holds OpenList passwords, but the import never reads
/// them, so each one is asked again and written where the config entry
/// points. A skipped one leaves its entry commented out.
fn openlist_passwords(
    prompt: &mut impl Prompt,
    roots: &BTreeMap<String, String>,
    old: &[OldRoot],
    data_dir: &Path,
) -> Result<()> {
    // The same order and first-wins rule as config_out::render, so the
    // password goes to the server of the entry that uses its file, never to
    // another library that shares the ID.
    let mut seen = std::collections::BTreeSet::new();
    for (name, id) in roots {
        // Only libraries render writes an entry for take part in the race.
        let Some(r) = old
            .iter()
            .find(|r| &r.name == name && matches!(r.kind.as_str(), "local" | "openlist"))
        else {
            continue;
        };
        if !seen.insert(id.as_str()) || r.kind != "openlist" {
            continue;
        }
        let place = r.url.as_deref().unwrap_or_default();
        let Some(pw) = prompt.password(
            &format!("OpenList 媒体库 {}（{place}）的密码", r.name),
            false,
        )?
        else {
            continue;
        };
        let path = data_dir
            .join("secrets")
            .join(format!("openlist-{id}.password"));
        config_out::write(&path, &pw)?;
    }
    Ok(())
}

/// Asks for the folder of a local library the import could not place.
fn ask_folder(prompt: &mut impl Prompt, root: &mut OldRoot, works: &[Work]) -> Result<()> {
    if let Some(old_path) = &root.old_path {
        prompt.say(&format!("  旧配置里的路径是 {old_path}"))?;
    }
    loop {
        let answer = prompt.ask(
            "  这个媒体库现在在哪个目录？（绝对路径，直接回车以后在配置里填）",
            Some(""),
        )?;
        if answer.is_empty() {
            return Ok(());
        }
        let path = PathBuf::from(&answer);
        if !(path.is_absolute() && path.is_dir()) {
            prompt.say("  这不是一个存在的绝对路径目录")?;
            continue;
        }
        root.path = Some(Located {
            path,
            origin: Origin::Given,
        });
        // A typed folder gets the same check as one from the old config.
        config_out::spot_check(std::slice::from_mut(root), works);
        let Some(s) = root.sample.filter(config_out::Sample::is_low) else {
            return Ok(());
        };
        let question = format!(
            "  抽查的 {} 部作品只在这里找到 {} 部，确认用这个目录吗？",
            s.checked, s.found
        );
        if prompt.confirm(&question, false)? {
            return Ok(());
        }
        root.path = None;
        root.sample = None;
    }
}

/// Where the media libraries go: the library's own config.toml when it has
/// none yet, otherwise a separate file the user merges by hand.
fn config_target(data_dir: &Path) -> PathBuf {
    let config = data_dir.join("config.toml");
    if config.exists() {
        data_dir.join("config.from-kikoeru.toml")
    } else {
        config
    }
}

fn has_journal(db: &Path) -> bool {
    let real = std::fs::canonicalize(db).unwrap_or_else(|_| db.to_owned());
    ["-journal", "-wal"].iter().any(|suffix| {
        let mut side = real.as_os_str().to_owned();
        side.push(suffix);
        std::fs::metadata(&side).is_ok_and(|m| m.len() > 0)
    })
}

fn preset_roots(pairs: &[String]) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for pair in pairs {
        let Some((old, new)) = pair.rsplit_once('=') else {
            bail!("--root {pair:?} is not OLD=NEW");
        };
        // Checked here, because --yes would offer the same invalid default
        // to the question forever.
        if !is_valid_root_id(new) {
            bail!(
                "--root {pair:?}: the new ID must be non-empty and use only ASCII letters, digits, '-' and '_'"
            );
        }
        map.insert(old.to_owned(), new.to_owned());
    }
    Ok(map)
}

/// An ID from the old name's ASCII letters and digits, lowercased, or
/// `rootN` for a name with none, such as 音声. It never repeats one already
/// chosen.
fn suggest_id(name: &str, taken: &BTreeMap<String, String>) -> String {
    let base: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .to_ascii_lowercase();
    let base = if base.is_empty() {
        "root".to_owned()
    } else {
        base
    };
    let used = |id: &str| taken.values().any(|v| v == id);
    if !base.starts_with("root") && !used(&base) {
        return base;
    }
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|id| !used(id))
        .unwrap_or(base)
}

/// Imported accounts have no password. Each one is offered in turn, and the
/// skipped ones are listed with the command that sets them later.
async fn set_passwords(data_dir: &Path, prompt: &mut impl Prompt) -> Result<()> {
    let data_dir = DataDir::existing(data_dir)?;
    let db = Db::open_sqlite(&data_dir.database()).await?;
    let result = async {
        let names = users::without_password(&db).await?;
        let mut skipped = Vec::new();
        for (name, admin) in names {
            let role = if admin { "管理员" } else { "用户" };
            let Some(pw) = prompt.password(&format!("{role} {name} 的新密码"), true)? else {
                skipped.push(name);
                continue;
            };
            let user = users::find_by_name(&db, &name)
                .await?
                .with_context(|| format!("no user named {name}"))?;
            users::set_password(&db, user.id, &user_cmd::hash(&pw)?).await?;
            prompt.say(&format!("已设置 {name} 的密码"))?;
        }
        if !skipped.is_empty() {
            prompt.say(&format!(
                "这些账号还没有密码，设置之前不能登录：{}\n设置方法：kikoeru-neo --data-dir {} user set-password <名字>",
                skipped.join("、"),
                data_dir.root().display()
            ))?;
        }
        anyhow::Ok(())
    }
    .await;
    db.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_ids_are_valid_and_distinct() {
        let mut taken = BTreeMap::new();
        for (name, want) in [
            ("Local", "local"),
            ("云端作品", "root1"),
            ("云端作品 2", "2"),
            ("Lo cal", "local1"),
        ] {
            let id = suggest_id(name, &taken);
            assert_eq!(id, want, "{name}");
            assert!(is_valid_root_id(&id));
            taken.insert(name.to_owned(), id);
        }
    }
}
