use std::{io::IsTerminal, path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::Parser;
use neo_db::Db;
use neo_server::{
    backup,
    cli::{Cli, Command},
    config::{self, Config, Source},
    datadir::DataDir,
    import, logging, migrate_cmd, serve, user_cmd,
};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: cannot start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let serving = matches!(&cli.command, Command::Serve(_));
    let result = runtime.block_on(run(cli));
    if serving {
        // stop() has already spent its shutdown budget. Runtime::drop would
        // wait indefinitely for workers stuck in synchronous filesystem I/O.
        runtime.shutdown_background();
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Under systemd the prefix files the message as an error, like
            // the log's own errors.
            eprintln!("{}", logging::error_lines(&format!("error: {e:#}")));
            if import::error::code(&e) == import::error::ErrorCode::WouldRefuse {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

/// The data directory and config a command reads, so that every command
/// opens the database `serve` uses: `config` (from `serve --config`), else
/// `KIKOERU_CONFIG`, else `$DATA_DIR/config.toml`, with
/// `KIKOERU_DATABASE_URL` over the file's `database.url`.
fn library(
    cli_data_dir: Option<PathBuf>,
    env_data_dir: Option<std::ffi::OsString>,
    config: Option<PathBuf>,
) -> Result<(config::Paths, Config, Source)> {
    let config = config.or_else(|| {
        std::env::var_os(config::CONFIG_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    let paths = config::resolve_paths(cli_data_dir, env_data_dir, config)?;
    let (config, source) = Config::load_with_env(&paths)?;
    Ok((paths, config, source))
}

async fn run(cli: Cli) -> Result<()> {
    let env_data_dir = std::env::var_os(config::DATA_DIR_ENV);
    match cli.command {
        Command::Serve(args) => {
            let (paths, config, source) = library(cli.data_dir, env_data_dir, args.config)?;
            let (config, replaced_listen) =
                config.with_listen_env(std::env::var_os(config::LISTEN_ENV))?;
            logging::init(config.log.level);
            if let Some(file) = replaced_listen {
                tracing::warn!(
                    file = %file,
                    listen = %config.listen,
                    "{} overrides listen from the config file",
                    config::LISTEN_ENV
                );
            }
            match source {
                Source::File => tracing::info!(path = %paths.config.display(), "loaded config"),
                Source::Defaults => tracing::info!(
                    path = %paths.config.display(),
                    "config file not found, using defaults"
                ),
            }
            // Nothing reads it yet: media and HLS URLs are relative, so a LAN
            // address over plain HTTP works without it and is not a fault.
            if config.public_url.is_none() {
                tracing::info!("public_url is not set");
            }
            let data_dir =
                DataDir::prepare_for(&paths.data_dir, &config.database(&paths.data_dir))?;
            serve::run(data_dir, config).await
        }
        Command::User(command) => {
            logging::init(tracing::Level::WARN);
            let (paths, config, _) = library(cli.data_dir, env_data_dir, None)?;
            let location = config.database(&paths.data_dir);
            DataDir::prepare_for(&paths.data_dir, &location)?;
            let db = Db::open(&location).await?;
            let result = user_cmd::run(&db, command).await;
            db.close().await;
            println!("{}", result?);
            Ok(())
        }
        Command::ImportLegacy(args) => {
            logging::init(tracing::Level::WARN);
            // A dry run writes nothing, so it needs no data directory.
            let target = if args.dry_run {
                None
            } else {
                let (paths, config, _) = library(cli.data_dir, env_data_dir, None)?;
                let location = config.database(&paths.data_dir);
                Some((paths.data_dir, location))
            };
            let cancel = cancel_on_signal();
            let result = import::run(
                &args,
                target.as_ref().map(|(dir, db)| (dir.as_path(), db)),
                &cancel,
                &mut std::io::stdout().lock(),
            )
            .await;
            // A would-refuse dry run already printed its report and refusal.
            if let Err(e) = &result
                && args.json
                && import::error::code(e) != import::error::ErrorCode::WouldRefuse
            {
                println!("{}", import::error::json(e));
            }
            result
        }
        Command::Migrate(args) => {
            logging::init(tracing::Level::WARN);
            let (paths, config, _) = library(cli.data_dir, env_data_dir, None)?;
            if !matches!(
                config.database(&paths.data_dir),
                neo_db::Location::Sqlite(_)
            ) {
                anyhow::bail!(
                    "migrate writes a SQLite library; run it without a PostgreSQL database \
                     configured, then move the library with `kikoeru-neo db copy`"
                );
            }
            let data_dir = paths.data_dir;
            let cancel = cancel_on_signal();
            let mut out = std::io::stdout().lock();
            if std::io::stdin().is_terminal() {
                migrate_cmd::run(
                    &args,
                    &data_dir,
                    &cancel,
                    &mut migrate_cmd::Terminal,
                    &mut out,
                )
                .await
            } else if args.yes {
                migrate_cmd::run(
                    &args,
                    &data_dir,
                    &cancel,
                    &mut migrate_cmd::Defaults,
                    &mut out,
                )
                .await
            } else {
                anyhow::bail!(
                    "migrate asks questions and needs a terminal; pass --yes to take every default, \
                     or use import-legacy with flags"
                )
            }
        }
        Command::Backup(args) => {
            logging::init(tracing::Level::WARN);
            let (paths, config, _) = library(cli.data_dir, env_data_dir, None)?;
            let data_dir = DataDir::existing(&paths.data_dir)?;
            let location = config.database(&paths.data_dir);
            let copied = backup::run(&data_dir, &location, &args.dest).await?;
            println!(
                "backed up {} files, {} bytes, to {}",
                copied.files,
                copied.bytes,
                args.dest.display()
            );
            if copied.skipped_links > 0 {
                println!("skipped {} symlinks", copied.skipped_links);
            }
            Ok(())
        }
        #[cfg(feature = "postgres")]
        Command::Db(neo_server::cli::DbCommand::Copy(args)) => {
            use neo_db::copy::{self, Endpoint};
            logging::init(tracing::Level::WARN);
            let from = match &args.from {
                Some(from) => Endpoint::parse(from),
                None => {
                    let (paths, config, _) = library(cli.data_dir, env_data_dir, None)?;
                    match config.database(&paths.data_dir) {
                        neo_db::Location::Sqlite(path) => {
                            DataDir::existing(&paths.data_dir)?;
                            Endpoint::Sqlite(path)
                        }
                        neo_db::Location::Postgres(url) => Endpoint::Postgres(url),
                    }
                }
            };
            let report = copy::copy(&from, &Endpoint::parse(&args.to)).await?;
            let mut total = 0;
            for (table, rows) in &report.tables {
                println!("{table:<20} {rows:>8}");
                total += rows;
            }
            println!("copied {total} rows");
            Ok(())
        }
    }
}

/// The first SIGINT or SIGTERM cancels the import, which then removes what
/// it wrote. A second one exits at once, for an import stuck somewhere that
/// does not check the token, and leaves the target to delete by hand.
///
/// The handlers are installed before this returns. Installing them inside
/// the spawned task left a window in which a signal still had its default
/// action and killed the import without cleanup.
fn cancel_on_signal() -> tokio_util::sync::CancellationToken {
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut signals = match Signals::install() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "warning: cannot handle signals, so an interrupted import is not cleaned up: {e}"
            );
            return cancel;
        }
    };
    let token = cancel.clone();
    tokio::spawn(async move {
        signals.recv().await;
        eprintln!("cancelling; the import removes what it wrote");
        token.cancel();
        signals.recv().await;
        eprintln!(
            "stopped at once: an unfinished merge changed nothing, but an unfinished \
             import into a new directory leaves it behind; delete only that new directory \
             before reusing it"
        );
        std::process::exit(130);
    });
    cancel
}

/// SIGINT and SIGTERM, registered once for the whole run so no signal falls
/// between two registrations.
#[cfg(unix)]
struct Signals {
    int: tokio::signal::unix::Signal,
    term: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn install() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            int: signal(SignalKind::interrupt())?,
            term: signal(SignalKind::terminate())?,
        })
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.int.recv() => {}
            _ = self.term.recv() => {}
        }
    }
}

#[cfg(not(unix))]
struct Signals(tokio::signal::windows::CtrlC);

#[cfg(not(unix))]
impl Signals {
    fn install() -> std::io::Result<Self> {
        tokio::signal::windows::ctrl_c().map(Self)
    }

    async fn recv(&mut self) {
        self.0.recv().await;
    }
}
