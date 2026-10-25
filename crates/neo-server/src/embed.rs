//! The entry point the desktop shell links against in local-library mode
//! (book/src/desktop.md, "外壳和后端的接口"). The shell owns logging and
//! process signals; this module only starts and stops the server.

use std::{
    ffi::OsString,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use neo_db::{
    self as db,
    users::{self, CreateError},
};

pub use crate::http::{WebAsset, WebSource};
use crate::{
    config::{self, Config, MediaConfig},
    datadir::DataDir,
    import::{
        merge::{self, Merged, UserMap},
        plan::Plan,
    },
    password,
    serve::{self, Running},
    token::{self, Audience},
    user_cmd,
};

const APP_DIR: &str = "kikoeru-neo";

pub struct Options {
    /// Usually [`default_data_dir`].
    pub data_dir: PathBuf,
    /// The ffmpeg and ffprobe bundled with the app, as absolute paths. Each
    /// one given overrides `media.ffmpeg` or `media.ffprobe` in
    /// `config.toml`; without either, the config applies, and without that
    /// `PATH` is searched.
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    /// The frontend's files. The app passes the copy Tauri embeds for the
    /// picker and builds neo-server without `embed-web`. Without either,
    /// page paths get the JSON 404.
    pub web: Option<WebSource>,
}

/// `kikoeru-neo` in the platform's application data directory, or `None`
/// when the environment does not say where that is.
pub fn default_data_dir() -> Option<PathBuf> {
    app_data_dir(std::env::consts::OS, |key| std::env::var_os(key))
}

/// [`default_data_dir`] for a given platform and environment.
pub fn app_data_dir(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = match os {
        "macos" => var("HOME")?.join("Library/Application Support"),
        "windows" => var("APPDATA")?,
        // The XDG spec says to ignore a relative XDG_DATA_HOME.
        _ => match var("XDG_DATA_HOME").filter(|p| p.is_absolute()) {
            Some(dir) => dir,
            None => var("HOME")?.join(".local/share"),
        },
    };
    Some(base.join(APP_DIR))
}

pub struct Server {
    running: Running,
}

/// Starts the server on the current tokio runtime and returns once it is
/// ready. It reads `config.toml` in the data directory like the server
/// build, except that it ignores `database.url` to use SQLite and always
/// listens on a random 127.0.0.1 port.
pub async fn start(options: Options) -> Result<Server> {
    let data_dir = DataDir::prepare(&options.data_dir)?;
    let paths = config::resolve_paths(Some(options.data_dir), None, None)?;
    let (mut config, _) = Config::load_sqlite(&paths)?;
    override_tools(&mut config.media, options.ffmpeg, options.ffprobe)?;
    // Other programs on the machine can still reach a loopback port, which
    // is why the desktop keeps users and tokens.
    config.listen = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    // People expect files added on their own machine to show up without a
    // scan, so local roots are watched unless the config says otherwise.
    config.watch_by_default = true;
    let running = serve::start(data_dir, config, options.web).await?;
    Ok(Server { running })
}

/// Applies the shell's bundled binaries over the config. They must be
/// absolute for the same reason as the config keys.
fn override_tools(
    media: &mut MediaConfig,
    ffmpeg: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
) -> Result<()> {
    for (name, path) in [("ffmpeg", &ffmpeg), ("ffprobe", &ffprobe)] {
        if let Some(path) = path
            && !path.is_absolute()
        {
            bail!(
                "the bundled {name} must be an absolute path, got {}",
                path.display()
            );
        }
    }
    if ffmpeg.is_some() {
        media.ffmpeg = ffmpeg;
    }
    if ffprobe.is_some() {
        media.ffprobe = ffprobe;
    }
    Ok(())
}

impl Server {
    pub fn port(&self) -> u16 {
        self.running.address().port()
    }

    /// Creates an administrator named `name` unless a user by that name
    /// exists. Returns whether one was created. The shell keeps the
    /// password in the system keychain.
    pub async fn ensure_admin(&self, name: &str, password: &str) -> Result<bool> {
        user_cmd::validate_name(name)?;
        if !password::is_long_enough(password) {
            bail!("the password is too short");
        }
        let hash = password::hash(password).map_err(|e| anyhow::anyhow!("{e}"))?;
        let db = self.running.state().db();
        match users::create(
            db,
            name,
            &hash,
            true,
            token::initial_version()?,
            db::now_ms(),
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(CreateError::NameTaken(_)) => Ok(false),
            Err(CreateError::Db(e)) => Err(e.into()),
        }
    }

    /// An API token for an existing user, the same as the login endpoint
    /// returns. There is no HTTP route for this, so the server build gains
    /// no way to sign in without a password.
    pub async fn session_token(&self, name: &str) -> Result<String> {
        let state = self.running.state();
        let user = users::find_by_name(state.db(), name)
            .await?
            .with_context(|| format!("no user named {name}"))?;
        let issued = state.tokens().issue(
            user.id,
            user.token_version,
            Audience::Api,
            token::now_secs(),
        )?;
        Ok(issued.token)
    }

    /// Merges an old library's favorites, reviews and play positions into
    /// this running library, as `import-legacy --merge` does. Build `plan`
    /// with `import::plan::build(&session.legacy, None, now)`. Works this
    /// library has not scanned yet get the data when a scan creates them.
    pub async fn merge_legacy(&self, plan: &Plan, users: &UserMap) -> Result<Merged> {
        // The shell has no way to cancel a merge, which is one short
        // transaction.
        let never = tokio_util::sync::CancellationToken::new();
        merge::merge(self.running.state().db(), plan, users, db::now_ms(), &never).await
    }

    /// Stops the server and waits until tasks have stopped and the
    /// database is closed.
    pub async fn stop(self) -> Result<()> {
        self.running.stop().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(ffmpeg: Option<&str>) -> MediaConfig {
        MediaConfig {
            heavy_jobs: 1,
            light_jobs: 1,
            job_timeout: std::time::Duration::from_secs(1),
            queue_limit: 1,
            ffmpeg: ffmpeg.map(PathBuf::from),
            ffprobe: None,
            video_transcode: true,
        }
    }

    #[test]
    fn bundled_tools_override_the_config_one_by_one() {
        let mut config = media(Some("/etc/ffmpeg"));
        override_tools(&mut config, None, Some("/app/ffprobe".into())).unwrap();
        assert_eq!(
            config.ffmpeg.as_deref(),
            Some(std::path::Path::new("/etc/ffmpeg"))
        );
        assert_eq!(
            config.ffprobe.as_deref(),
            Some(std::path::Path::new("/app/ffprobe"))
        );
        override_tools(&mut config, Some("/app/ffmpeg".into()), None).unwrap();
        assert_eq!(
            config.ffmpeg.as_deref(),
            Some(std::path::Path::new("/app/ffmpeg"))
        );
        assert!(override_tools(&mut config, Some("ffmpeg".into()), None).is_err());
    }
}
