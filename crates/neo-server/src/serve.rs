use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::serve::ListenerExt;
use neo_db::{Db, roots, server_settings};
use neo_media::TranscodeCache;
use neo_scanner::metadata::{
    self, AsmrOneConfig, AsmrOneProvider, CoverDirs, DlsiteConfig, DlsiteProvider, HvdbConfig,
    HvdbProvider, ProviderChain, ProxyChoice,
};
use neo_storage::Storage;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::{
    autoscan,
    config::{Config, MetadataConfig, MetadataFallback, MetadataSource},
    datadir::DataDir,
    http::{self, AppState, Media, RootEntry, WebSource},
    roots as library,
    scan::{self, MetadataEnv},
    tasks::{Lane, Scheduler},
    token::Tokens,
    watch,
};

/// How long running tasks get to stop their subprocesses at shutdown.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// How long open connections get to finish their responses at shutdown. A
/// client that stops reading would otherwise hold `stop` forever. Together
/// with [`HTTP_CLOSE`] and [`SHUTDOWN_GRACE`] this stays under a 30 s
/// container stop timeout.
const HTTP_DRAIN: Duration = Duration::from_secs(10);

/// How long closed connections get to unwind after the drain timeout. A
/// connection only notices on its next read or write.
const HTTP_CLOSE: Duration = Duration::from_secs(2);
/// An analysis of a remote file that plays directly waits this long before
/// it copies the file, so it does not compete with the player's first
/// buffering (book/src/media.md, "响度和波形").
const REMOTE_ANALYSIS_DELAY: Duration = Duration::from_secs(20);

mod killable;

/// Serves until SIGINT or SIGTERM. Any startup error, a failed migration
/// included, is returned so the process exits non-zero instead of running
/// half-started.
pub async fn run(data_dir: DataDir, config: Config) -> Result<()> {
    let running = start(data_dir, config, None).await?;
    tokio::select! {
        () = shutdown_signal() => {}
        // Only a failing listener ends the server on its own.
        () = running.ended.cancelled() => {}
    }
    running.stop().await
}

/// A server started by [`start`] and serving on its own task.
pub struct Running {
    address: SocketAddr,
    state: AppState,
    scheduler: Scheduler,
    db: Db,
    stop: CancellationToken,
    /// Closes every open connection, for when draining takes too long.
    kill: CancellationToken,
    ended: CancellationToken,
    serving: tokio::task::JoinHandle<std::io::Result<()>>,
    /// Stops the automatic-scan scheduler and the directory watchers.
    automation: CancellationToken,
    background: Vec<tokio::task::JoinHandle<()>>,
    /// See [`DataDir::lock_instance`]. Cleanup retains it after `stop` times
    /// out, until the jobs and the database have actually finished.
    _instance: std::fs::File,
}

impl Running {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn state(&self) -> &AppState {
        &self.state
    }

    /// Stops accepting requests, gives open connections and running tasks
    /// their grace periods, then closes the database within the remaining
    /// shutdown budget.
    pub async fn stop(self) -> Result<()> {
        self.stop_within(HTTP_DRAIN).await
    }

    async fn stop_within(mut self, drain: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + drain + HTTP_CLOSE + SHUTDOWN_GRACE;
        let root_edits = self.state.close_root_edits();
        // Tasks are told at once, not after every connection has drained,
        // which a slow client could put off until the drain timeout.
        self.scheduler.begin_shutdown();
        self.automation.cancel();
        self.stop.cancel();
        let mut served = tokio::time::timeout(drain, &mut self.serving).await.ok();
        if served.is_none() {
            tracing::warn!(
                ?drain,
                "open connections did not finish in time, closing them"
            );
            self.kill.cancel();
            served = tokio::time::timeout(HTTP_CLOSE, &mut self.serving)
                .await
                .ok();
            if served.is_none() {
                // A connection that has not touched its socket since has not
                // noticed yet. It closes on its next read or write.
                tracing::warn!("open connections did not close, no longer waiting for them");
                self.serving.abort();
            }
        }
        for task in self.background.drain(..) {
            let _ = task.await;
        }
        let grace =
            SHUTDOWN_GRACE.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        cleanup_within(
            self.scheduler,
            self.db,
            self._instance,
            root_edits,
            grace,
            deadline,
        )
        .await?;
        tracing::info!("stopped");
        if let Some(served) = served {
            served.context("the server task panicked")??;
        }
        Ok(())
    }
}

async fn cleanup_within(
    scheduler: Scheduler,
    db: Db,
    instance: std::fs::File,
    root_edits: tokio_util::task::TaskTracker,
    grace: Duration,
    deadline: tokio::time::Instant,
) -> Result<()> {
    // Cancellation cannot preempt synchronous I/O or destructors. Keep
    // their cleanup and instance lock alive if the caller stops waiting.
    let cleanup = tokio::spawn(async move {
        scheduler.shutdown(grace).await;
        root_edits.wait().await;
        db.close().await;
        drop(instance);
    });
    match tokio::time::timeout_at(deadline, cleanup).await {
        Ok(result) => result.context("server cleanup panicked"),
        Err(_) => {
            tracing::warn!("server cleanup exceeded the shutdown budget; continuing in background");
            Ok(())
        }
    }
}

/// Opens everything the server needs, binds the listener and starts
/// serving on a new task. Returns once the server is ready. `web` replaces
/// the frontend compiled in with `embed-web`, see [`WebSource`].
pub async fn start(data_dir: DataDir, config: Config, web: Option<WebSource>) -> Result<Running> {
    let instance = data_dir.lock_instance()?;
    data_dir.clear_tmp()?;
    let key = data_dir.load_or_create_jwt_key()?;
    let db = Db::open(&config.database(data_dir.root())).await?;
    // cache/tmp is on the same file system as covers, so publishing a
    // cover is a rename.
    let cover_dirs = CoverDirs::new(
        data_dir.root().join("covers"),
        data_dir.root().join("cache/tmp"),
    );
    // Before anything reads a cover, so no file is served under the
    // wrong cover_version.
    metadata::recover_cover_swaps(&db, &cover_dirs)
        .await
        .context("cannot recover interrupted cover updates")?;
    let loaded = load_roots(&db, data_dir.root(), &config).await?;
    let scheduler = Scheduler::new(
        db.clone(),
        config.media.heavy_jobs as usize,
        config.media.light_jobs as usize,
    );
    let media = Media {
        cache: TranscodeCache::open(
            &data_dir.root().join("cache"),
            config.cache.max_bytes,
            config.cache.min_free_bytes,
        )
        .context("cannot open the transcode cache")?
        .with_tools(config.media.tools()),
        probe_slots: Arc::new(Semaphore::new(config.media.light_jobs as usize)),
        queue_limit: config.media.queue_limit as usize,
        job_timeout: config.media.job_timeout,
        remote_analysis_delay: REMOTE_ANALYSIS_DELAY,
        video_transcode: config.media.video_transcode,
    };
    let metadata = match config.metadata.source {
        MetadataSource::Dlsite => Some(MetadataEnv {
            provider: Arc::new(metadata_chain(&config.metadata, &stored_proxy(&db).await?)?),
            dirs: cover_dirs.clone(),
        }),
        MetadataSource::None => None,
    };
    let state = AppState::new(
        db.clone(),
        Tokens::new(&key),
        config.trusted_proxies.clone(),
    )
    .with_data_dir(data_dir.clone())
    .with_configured_passwords(library::configured_passwords(&config.roots))
    .with_watch_by_default(config.watch_by_default)
    .with_roots(loaded.entries)
    .with_scheduler(scheduler.clone())
    .with_storages(loaded.storages)
    .with_media(media)
    .with_covers(cover_dirs);
    let state = match metadata {
        Some(metadata) => state.with_metadata(metadata),
        None => state,
    };
    let state = match web {
        Some(web) => state.with_web(web),
        None => state,
    };
    // A bind failure must leave queued tasks untouched for the next start.
    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let address = listener.local_addr()?;
    let scan_env = state.scan_env();
    // Queued scans, transcodes, probes and analyses run again after a
    // restart. Other leftovers are marked interrupted.
    if let Err(error) = scheduler
        .recover(|row| {
            http::rebuild_task(&state, row)
                .or_else(|| http::rebuild_metadata_task(&state, row))
                .or_else(|| {
                    if row.kind != scan::KIND {
                        return None;
                    }
                    let root = scan::payload_root(&row.payload)?;
                    let storage = state.storage(&root)?;
                    let (_, kind) = state.root_kinds().into_iter().find(|(id, _)| *id == root)?;
                    let env = scan_env.clone().with_full(scan::payload_full(&row.payload));
                    let job = scan::job(env, root, storage, kind);
                    Some((Lane::Scan, scan::TIMEOUT, job))
                })
        })
        .await
    {
        // Recovery may have spawned jobs before a later row failed.
        let deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE + HTTP_CLOSE;
        if let Err(cleanup_error) = cleanup_within(
            scheduler,
            db,
            instance,
            state.close_root_edits(),
            SHUTDOWN_GRACE,
            deadline,
        )
        .await
        {
            tracing::warn!(%cleanup_error, "cannot finish cleanup after failed recovery");
        }
        return Err(error.into());
    }
    // Only once nothing below can fail: start returning an error must not
    // leave these running on the caller's runtime.
    let automation = CancellationToken::new();
    let background = vec![
        tokio::spawn(autoscan::run(state.clone(), automation.clone())),
        tokio::spawn(watch::run(state.clone(), automation.clone())),
    ];
    let app = http::router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    state.set_ready();
    tracing::info!(%address, "listening");
    let stop = CancellationToken::new();
    let kill = CancellationToken::new();
    let ended = CancellationToken::new();
    // tap_io is a no-op here. axum provides ConnectInfo<SocketAddr> for a
    // tapped listener of any type, but for an untapped one only for
    // TcpListener itself.
    let listener = killable::KillableListener::new(listener, kill.clone()).tap_io(|_| {});
    let serving = tokio::spawn({
        let stop = stop.clone();
        let ended = ended.clone();
        async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await;
            ended.cancel();
            served
        }
    });
    Ok(Running {
        address,
        state,
        scheduler,
        db,
        stop,
        kill,
        ended,
        serving,
        automation,
        background,
        _instance: instance,
    })
}

/// The proxy saved on the settings page. A value that no longer parses,
/// which only a hand-edited database has, falls back to the environment
/// rather than keeping the server down.
async fn stored_proxy(db: &Db) -> Result<ProxyChoice> {
    let stored = server_settings::metadata_proxy(db)
        .await
        .context("cannot read the server settings")?;
    let proxy = ProxyChoice::from_setting(stored.as_deref()).unwrap_or_else(|reason| {
        tracing::warn!(%reason, "ignoring the stored metadata proxy");
        ProxyChoice::Environment
    });
    if let ProxyChoice::Fixed(url) = &proxy {
        tracing::info!(
            proxy = url.as_str(),
            "metadata requests go through the saved proxy"
        );
    }
    Ok(proxy)
}

/// DLsite, then asmr.one when `metadata.fallback` asks for it, then hvdb
/// for missing voice actors when `metadata.hvdb` does.
fn metadata_chain(config: &MetadataConfig, proxy: &ProxyChoice) -> Result<ProviderChain> {
    let mut dlsite = DlsiteConfig {
        proxy: proxy.clone(),
        ..DlsiteConfig::default()
    };
    if let Some(base) = &config.dlsite_url {
        dlsite.base.clone_from(base);
    }
    let dlsite = DlsiteProvider::new(dlsite)
        .map_err(|e| anyhow::anyhow!("cannot set up the DLsite client: {e}"))?;
    let fallback: Option<Arc<dyn metadata::MetadataProvider>> = match config.fallback {
        MetadataFallback::None => None,
        MetadataFallback::AsmrOne => {
            let mut asmr = AsmrOneConfig {
                proxy: proxy.clone(),
                ..AsmrOneConfig::default()
            };
            if let Some(mirrors) = &config.asmr_one_urls {
                asmr.mirrors.clone_from(mirrors);
            }
            Some(Arc::new(AsmrOneProvider::new(asmr).map_err(|e| {
                anyhow::anyhow!("cannot set up the asmr.one client: {e}")
            })?))
        }
    };
    let chain = ProviderChain::new(Arc::new(dlsite), fallback);
    if !config.hvdb {
        return Ok(chain);
    }
    let mut hvdb = HvdbConfig {
        proxy: proxy.clone(),
        ..HvdbConfig::default()
    };
    if let Some(base) = &config.hvdb_url {
        hvdb.base.clone_from(base);
    }
    let hvdb = HvdbProvider::new(hvdb)
        .map_err(|e| anyhow::anyhow!("cannot set up the hvdb client: {e}"))?;
    Ok(chain.with_voice_actors(Arc::new(hvdb)))
}

/// The roots the server starts with and the storage of each one it can
/// read.
struct LoadedRoots {
    entries: Vec<RootEntry>,
    storages: Vec<(String, Arc<dyn Storage>)>,
}

/// Merges the roots in config.toml with those added through the API.
/// A config root that cannot be built stops the server, as before roots
/// could be added at runtime. A stored root that cannot be built is still
/// listed but has no storage, so one bad password file does not keep the
/// whole library offline.
async fn load_roots(db: &Db, data_dir: &Path, config: &Config) -> Result<LoadedRoots> {
    let secrets_root = data_dir.to_owned();
    let _secrets =
        tokio::task::spawn_blocking(move || library::lock_secrets(&secrets_root)).await??;
    if let Err(e) =
        library::cleanup_passwords(data_dir, db, &library::configured_passwords(&config.roots))
            .await
    {
        tracing::warn!("cannot clean up unreferenced password files: {e:#}");
    }
    let names: Vec<_> = config
        .roots
        .iter()
        .map(|r| (r.id.as_str(), r.display_name.as_str()))
        .collect();
    roots::sync(db, &names)
        .await
        .context("cannot record the configured roots")?;
    let mut loaded = LoadedRoots {
        entries: Vec::new(),
        storages: Vec::new(),
    };
    for root in &config.roots {
        loaded
            .entries
            .push(library::config_entry(root, config.watch_by_default));
        let storage = library::build_storage(&root.id, &root.backend)?;
        loaded.storages.push((root.id.clone(), storage));
    }
    let stored = roots::list_stored(db)
        .await
        .context("cannot read the roots added through the API")?;
    let known: Vec<&str> = config
        .roots
        .iter()
        .map(|r| r.id.as_str())
        .chain(stored.iter().map(|r| r.id.as_str()))
        .collect();
    // Only a warning: the works keep their rows, and adding the root back
    // makes them playable again without a new import.
    match roots::with_works_outside(db, &known).await {
        Ok(orphans) => {
            for (root, works) in orphans {
                tracing::warn!(
                    %root,
                    works,
                    "the library has works in this root, but neither config.toml nor the \
                     settings page defines it; add it to play them"
                );
            }
        }
        Err(e) => tracing::warn!("cannot check for roots missing from the config: {e:#}"),
    }
    for root in stored {
        if config.roots.iter().any(|r| r.id == root.id) {
            tracing::warn!(
                root = %root.id,
                "config.toml and the database both define this root, using config.toml"
            );
            continue;
        }
        let (entry, backend) = match library::stored_entry(data_dir, &root, config.watch_by_default)
        {
            Ok(built) => built,
            Err(e) => {
                tracing::warn!(root = %root.id, "skipping a stored root: {e:#}");
                continue;
            }
        };
        match library::build_storage(&root.id, &backend) {
            Ok(storage) => loaded.storages.push((root.id.clone(), storage)),
            Err(e) => tracing::warn!(root = %root.id, "root is unavailable: {e:#}"),
        }
        loaded.entries.push(entry);
    }
    Ok(loaded)
}

#[cfg(unix)]
async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler can be installed");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

/// Windows has no SIGTERM. The desktop app stops the server through
/// `embed`, so only Ctrl+C in a console is left.
#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use neo_db::users;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::*;
    use crate::token::{self, Audience};

    /// Larger than the socket buffers on both ends of a loopback connection
    /// can hold, so the response write blocks.
    const TRACK_BYTES: u64 = 64 << 20;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_bounds_synchronous_job_io_and_keeps_the_instance_locked() {
        stop_with_blocked_job(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_bounds_synchronous_job_drop_and_keeps_the_instance_locked() {
        stop_with_blocked_job(true).await;
    }

    async fn stop_with_blocked_job(block_on_drop: bool) {
        struct Cleanup {
            release: Option<std::sync::mpsc::Receiver<()>>,
            partial: std::path::PathBuf,
        }

        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(release) = &self.release {
                    let _ = release.recv();
                }
                std::fs::remove_file(&self.partial).unwrap();
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let data_dir = DataDir::prepare(&dir.path().join("data")).unwrap();
        let database = neo_db::testing::open().await;
        let database_config = match &database.location {
            neo_db::Location::Sqlite(_) => String::new(),
            #[cfg(feature = "postgres")]
            neo_db::Location::Postgres(url) => format!("[database]\nurl = {url:?}\n"),
        };
        let config = Config::parse(
            &format!("listen = \"127.0.0.1:0\"\n[metadata]\nsource = \"none\"\n{database_config}"),
            Path::new("config.toml"),
        )
        .unwrap();
        database.db.close().await;
        let running = start(data_dir.clone(), config, None).await.unwrap();
        let db = running.db.clone();
        let partial = data_dir.root().join("cache/tmp/blocked-job");
        std::fs::write(&partial, b"partial").unwrap();
        // Dropping the sender also releases the job if a test assertion fails.
        let (release, blocked) = std::sync::mpsc::channel();
        let cleanup = Cleanup {
            release: None,
            partial: partial.clone(),
        };
        let (started, ready) = tokio::sync::oneshot::channel();
        running
            .scheduler
            .submit(
                crate::tasks::TaskSpec {
                    kind: "metadata",
                    dedupe_key: None,
                    payload: serde_json::json!({}),
                    lane: Lane::Light,
                    timeout: None,
                    queue_limit: None,
                },
                Box::new(move |_| {
                    Box::pin(async move {
                        let mut cleanup = cleanup;
                        if block_on_drop {
                            cleanup.release = Some(blocked);
                            started.send(()).unwrap();
                        } else {
                            started.send(()).unwrap();
                            // Like sync_all(), this cannot poll cancellation.
                            let _ = blocked.recv();
                        }
                        std::future::pending().await
                    })
                }),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();

        let stopped = tokio::time::timeout(
            HTTP_CLOSE + SHUTDOWN_GRACE + Duration::from_secs(3),
            running.stop_within(Duration::ZERO),
        )
        .await;
        let still_locked = data_dir.lock_instance().is_err();
        let still_partial = partial.exists();
        release.send(()).unwrap();
        // Let both the blocked operation and its destructor finish before
        // reporting failure, so the test runtime can shut down too.
        tokio::time::timeout(Duration::from_secs(5), async {
            while partial.exists() || data_dir.lock_instance().is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the old instance did not finish cleaning up");
        db.close().await;
        stopped
            .expect("stop waited indefinitely for synchronous job I/O or Drop")
            .unwrap();
        assert!(
            still_locked,
            "stop released the lock while the job was alive"
        );
        assert!(
            still_partial,
            "stop discarded the job's cleanup responsibility"
        );
    }

    #[tokio::test]
    async fn stop_bounds_database_close_and_drops_stuck_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = DataDir::prepare(&dir.path().join("data")).unwrap();
        let database = neo_db::testing::open().await;
        let database_config = match &database.location {
            neo_db::Location::Sqlite(_) => String::new(),
            #[cfg(feature = "postgres")]
            neo_db::Location::Postgres(url) => format!("[database]\nurl = {url:?}\n"),
        };
        let config = Config::parse(
            &format!("listen = \"127.0.0.1:0\"\n[metadata]\nsource = \"none\"\n{database_config}"),
            Path::new("config.toml"),
        )
        .unwrap();
        database.db.close().await;
        let running = start(data_dir, config, None).await.unwrap();
        let db = running.db.clone();
        let job_db = db.clone();
        let dropped = CancellationToken::new();
        let guard = dropped.clone().drop_guard();
        let (started, ready) = tokio::sync::oneshot::channel();
        running
            .scheduler
            .submit(
                crate::tasks::TaskSpec {
                    kind: "scan",
                    dedupe_key: None,
                    payload: serde_json::json!({}),
                    lane: Lane::Scan,
                    timeout: None,
                    queue_limit: None,
                },
                Box::new(move |ctx| {
                    Box::pin(async move {
                        let _guard = guard;
                        // A query whose peer stopped responding holds a
                        // connection and cannot observe cooperative cancel.
                        neo_db::with_pool!(&job_db, write, pool => {
                            let _connection = pool.acquire().await.unwrap();
                            started.send(ctx.progress().clone()).unwrap();
                            std::future::pending().await
                        })
                    })
                }),
            )
            .await
            .unwrap();
        let progress = tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .expect("the job did not acquire its connection")
            .unwrap();

        // A second borrower outlives the job, so cancelling jobs alone
        // cannot make pool closure finish within the shutdown budget.
        neo_db::with_pool!(&db, read, pool => {
            let _connection = pool.acquire().await.unwrap();
            tokio::time::timeout(
                HTTP_CLOSE + SHUTDOWN_GRACE + Duration::from_secs(3),
                running.stop_within(Duration::ZERO),
            )
            .await
            .expect("stop waited indefinitely for a borrowed database connection")
            .unwrap();
        });
        assert!(dropped.is_cancelled(), "the stuck job survived stop");
        assert!(progress.is_closed(), "the progress writer survived stop");
        db.close().await;
    }

    #[tokio::test]
    async fn roots_added_through_the_api_come_back_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let extra = dir.path().join("extra");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        let data_dir = DataDir::prepare(&dir.path().join("data")).unwrap();
        let db = Db::open_sqlite(&data_dir.database()).await.unwrap();
        let stored = [
            library::StoredSettings::Local {
                path: extra.clone(),
                watch: None,
                auto_scan_interval: None,
            }
            .to_stored("extra", "Extra"),
            // Shadowed by the config root of the same id.
            library::StoredSettings::Local {
                path: extra.clone(),
                watch: None,
                auto_scan_interval: None,
            }
            .to_stored("lib", "Stored lib"),
            // Its password file is gone, so it is listed without storage.
            library::StoredSettings::OpenList {
                url: "http://127.0.0.1:9".into(),
                base_path: "/".into(),
                username: "u".into(),
                password_version: None,
                list_concurrency: None,
                link_rate: None,
                auto_scan_interval: None,
            }
            .to_stored("remote", "Remote"),
        ];
        for root in &stored {
            assert!(roots::insert_stored(&db, root, 0).await.unwrap());
        }
        db.close().await;
        let config = Config::parse(
            &format!(
                "listen = \"127.0.0.1:0\"\n\
                 [[roots]]\nid = \"lib\"\ndisplay_name = \"Lib\"\ntype = \"local\"\npath = {:?}\n\
                 [metadata]\nsource = \"none\"\n",
                library.display().to_string()
            ),
            Path::new("config.toml"),
        )
        .unwrap();
        let running = start(data_dir, config, None).await.unwrap();
        let state = running.state();
        let listed: Vec<_> = state
            .roots()
            .into_iter()
            .map(|r| (r.id, r.source))
            .collect();
        assert_eq!(
            listed,
            [
                ("lib".to_owned(), neo_core::dto::RootSource::Config),
                ("extra".to_owned(), neo_core::dto::RootSource::Db),
                ("remote".to_owned(), neo_core::dto::RootSource::Db),
            ]
        );
        assert!(state.storage("lib").is_some());
        assert!(state.storage("extra").is_some());
        assert!(state.storage("remote").is_none());
        running.stop().await.unwrap();
    }

    #[tokio::test]
    async fn a_second_server_on_the_same_data_directory_leaves_the_first_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let config = || {
            Config::parse(
                "listen = \"127.0.0.1:0\"\n[metadata]\nsource = \"none\"\n",
                Path::new("config.toml"),
            )
            .unwrap()
        };
        let first = start(DataDir::prepare(&root).unwrap(), config(), None)
            .await
            .unwrap();
        // A transcode of the first server, still being written.
        let partial = root.join("cache/tmp/job-1.m4a");
        std::fs::write(&partial, b"partial").unwrap();
        let second = start(DataDir::prepare(&root).unwrap(), config(), None).await;
        let error = format!(
            "{:#}",
            second.err().expect("the second server must not start")
        );
        assert!(error.contains("another kikoeru-neo server"), "{error}");
        assert!(partial.exists(), "the first server's files are untouched");
        first.stop().await.unwrap();
        // Once the first one has stopped, the directory is free again.
        let again = start(DataDir::prepare(&root).unwrap(), config(), None)
            .await
            .unwrap();
        again.stop().await.unwrap();
    }

    #[tokio::test]
    async fn stop_does_not_wait_for_a_client_that_never_reads() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        std::fs::create_dir_all(library.join("RJ001")).unwrap();
        std::fs::File::create(library.join("RJ001/01.mp3"))
            .unwrap()
            .set_len(TRACK_BYTES)
            .unwrap();
        let data_dir = DataDir::prepare(&dir.path().join("data")).unwrap();
        let config = Config::parse(
            &format!(
                "listen = \"127.0.0.1:0\"\n\
                 [[roots]]\nid = \"lib\"\ndisplay_name = \"Lib\"\ntype = \"local\"\npath = {:?}\n\
                 [metadata]\nsource = \"none\"\n",
                library.display().to_string()
            ),
            Path::new("config.toml"),
        )
        .unwrap();
        let running = start(data_dir, config, None).await.unwrap();
        let version = token::initial_version().unwrap();
        let user = users::create(&running.db, "alice", "unused", false, version, 0)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
               VALUES (1, 'RJ000001', 'Work', 'scan', 0, 0);
             INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
               VALUES (1, 1, 'lib', CAST('RJ001' AS BLOB), 'RJ001');
             INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind)
               VALUES (1, 1, 1, CAST('01.mp3' AS BLOB), '01.mp3', 'a', 'audio');",
        )
        .execute(&running.db.sqlite().unwrap().write)
        .await
        .unwrap();
        let bearer = running
            .state()
            .tokens()
            .issue(user, version, Audience::Api, token::now_secs())
            .unwrap()
            .token;

        // A player that paused: the stream request is sent and the
        // response is never read past its status line.
        let mut client = TcpStream::connect(running.address()).await.unwrap();
        client
            .write_all(
                format!(
                    "GET /api/tracks/1/stream HTTP/1.1\r\nHost: x\r\n\
                     Authorization: Bearer {bearer}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut status = [0u8; 12];
        client.read_exact(&mut status).await.unwrap();
        assert_eq!(&status, b"HTTP/1.1 200");
        tokio::time::sleep(Duration::from_millis(300)).await;

        let drain = Duration::from_millis(500);
        let stopped = tokio::time::timeout(Duration::from_secs(5), running.stop_within(drain))
            .await
            .expect("stop waited for the client");
        stopped.unwrap();

        // The runtime keeps running, as it does around an embedded server,
        // so a connection task left behind would go on to send the whole
        // track. The client gets what the socket buffers held, then EOF.
        let mut received = status.len() as u64;
        let mut buf = vec![0u8; 1 << 16];
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match client.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => received += n as u64,
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "the connection stayed open after stop");
        assert!(
            received < TRACK_BYTES,
            "the connection kept streaming after stop: {received} bytes"
        );
    }
}
