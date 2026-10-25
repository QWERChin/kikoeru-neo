//! The kikoeru-neo server: command line, configuration and HTTP API.

pub mod autoscan;
pub mod backup;
pub mod cli;
pub mod config;
pub mod datadir;
pub mod embed;
pub mod http;
pub mod import;
pub mod logging;
pub mod migrate_cmd;
pub mod password;
pub mod private;
pub mod roots;
pub mod scan;
pub mod serve;
pub mod tasks;
pub mod token;
pub mod user_cmd;
pub mod watch;

use neo_core::dto::VersionResponse;

/// Build information comes from the build environment. The Nix package sets
/// both variables. A plain `cargo build` leaves them unset, so they are null.
pub fn version() -> VersionResponse {
    VersionResponse {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_commit: option_env!("KIKOERU_GIT_COMMIT")
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        build_time: option_env!("KIKOERU_BUILD_TIME")
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|ms| ms.try_into().ok()),
    }
}
