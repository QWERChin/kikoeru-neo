use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "kikoeru-neo",
    version,
    about = "Self-hosted player for audio dramas and audiobooks"
)]
pub struct Cli {
    /// Data directory. Falls back to $KIKOERU_DATA_DIR.
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP server.
    Serve(ServeArgs),
    /// Manage user accounts on this server.
    #[command(subcommand)]
    User(UserCommand),
    /// Import an old kikoeru data directory into a new, empty data directory.
    ImportLegacy(ImportArgs),
    /// Import an old kikoeru install step by step, asking in Chinese.
    Migrate(MigrateArgs),
    /// Copy the database and the state directories into a new directory.
    /// Safe while the server is running.
    Backup(BackupArgs),
    /// Move the library database between SQLite and PostgreSQL.
    #[cfg(feature = "postgres")]
    #[command(subcommand)]
    Db(DbCommand),
}

#[cfg(feature = "postgres")]
#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Copy every row into a new SQLite file or an empty PostgreSQL
    /// database. The source is only read. Stop the server first: what it
    /// writes after the copy starts stays behind in the source.
    Copy(DbCopyArgs),
}

#[cfg(feature = "postgres")]
#[derive(Debug, Args)]
pub struct DbCopyArgs {
    /// A SQLite file or a postgres:// URL. Defaults to the database in the
    /// data directory.
    #[arg(long, value_name = "FILE|URL")]
    pub from: Option<String>,
    /// A SQLite file that does not exist yet, or a postgres:// URL of an
    /// empty database. A password belongs in PGPASSWORD or ~/.pgpass, where
    /// the process list and shell history cannot see it.
    #[arg(long, value_name = "FILE|URL")]
    pub to: String,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Where to write the backup. It must not exist yet.
    #[arg(value_name = "DEST")]
    pub dest: PathBuf,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The old data directory, or its sqlite/ or config/ folder, its
    /// db.sqlite3, or the Docker volume directory that holds it.
    #[arg(long, value_name = "PATH")]
    pub source: PathBuf,
    /// The old database, when it is not at sqlite/db.sqlite3 in the source.
    #[arg(long, value_name = "FILE")]
    pub db: Option<PathBuf>,
    /// The old covers folder, when it is not covers/ in the source.
    #[arg(long, value_name = "DIR")]
    pub covers: Option<PathBuf>,
    /// The old config.json, when it is not in config/ of the source. Only
    /// paths, the database type and the media library folders are read.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Map a folder named in the old config to where it is now, such as
    /// /usr/src/kikoeru/VoiceWork=/srv/voice. Repeat for each folder.
    #[arg(long = "map-path", value_name = "OLD=NEW")]
    pub map_paths: Vec<String>,
    /// Use paths that were inferred from a neighbouring folder.
    #[arg(long)]
    pub accept_guessed: bool,
    /// When the old database has a non-empty journal or WAL, let SQLite
    /// recover the private copy instead of refusing. The source is not
    /// changed either way.
    #[arg(long)]
    pub recover_journal: bool,
    /// Only read the source and print the preflight summary. Exits with 2
    /// when the import would refuse to run.
    #[arg(long)]
    pub dry_run: bool,
    /// With --dry-run, print the report as JSON instead of text.
    #[arg(long, requires = "dry_run")]
    pub json: bool,
    /// Map an old root_folder name to a new root ID, such as Local=local.
    /// Repeat for every root_folder.
    /// With --merge, the IDs only name the entries --write-config writes.
    #[arg(long = "root", value_name = "OLD=NEW")]
    pub roots: Vec<String>,
    /// Write [[roots]] for the old media library folders, mapped with
    /// --root, into this new file for merging into config.toml.
    #[arg(long, value_name = "FILE")]
    pub write_config: Option<PathBuf>,
    /// Merge only favorites, reviews and play positions into the existing
    /// library in --data-dir. Its works come from its own scans.
    #[arg(long)]
    pub merge: bool,
    /// With --merge, merge an old user into an account with another name,
    /// such as admin=me. Other old users keep their name.
    #[arg(long = "user", value_name = "OLD=NEW", requires = "merge")]
    pub users: Vec<String>,
}

#[derive(Debug, Args)]
pub struct MigrateArgs {
    /// The old install. Asked for when not given.
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,
    /// Map a folder named in the old config to where it is now. Repeat as
    /// needed.
    #[arg(long = "map-path", value_name = "OLD=NEW")]
    pub map_paths: Vec<String>,
    /// The answer for an old media library's new ID, such as Local=local.
    #[arg(long = "root", value_name = "OLD=NEW")]
    pub roots: Vec<String>,
    /// Take every default without asking, for use without a terminal.
    /// No passwords are set then.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Config file. Defaults to $DATA_DIR/config.toml.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Create a user. Use --admin for the first administrator.
    Create {
        name: String,
        #[arg(long)]
        admin: bool,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Set or reset a password. Revokes all of the user's tokens.
    SetPassword {
        name: String,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Delete a user. The last administrator cannot be deleted.
    Delete { name: String },
}

#[derive(Debug, Args)]
pub struct PasswordSource {
    /// Read the password from the first line of stdin instead of prompting.
    #[arg(long)]
    pub password_stdin: bool,
}
