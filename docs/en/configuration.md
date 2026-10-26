# Configure the server

The server reads a TOML configuration file and stores accounts, library state and playback progress in its database. Identify the configuration and data directory your service actually uses before changing them. File settings are read at startup, so changes require a restart. Check [scan tasks](upgrade.md) before restarting.

## Select the configuration and data directory

| Setting | Precedence, highest first |
|---|---|
| Data directory | `--data-dir`, then nonempty `KIKOERU_DATA_DIR`. If neither is set, the command fails rather than using the current directory. |
| Configuration file | `serve --config`, then nonempty `KIKOERU_CONFIG`, then `config.toml` in the data directory. |
| PostgreSQL URL | Nonempty `KIKOERU_DATABASE_URL`, then `database.url`. With neither set, the server uses `neo.sqlite3` in the data directory. |
| Listener | Nonempty `KIKOERU_LISTEN`, then `listen`, then `127.0.0.1:8888`. Only `serve` reads this environment variable. |
| Log level | `RUST_LOG`, then `log.level`, then `info`. |

A missing `config.toml` at the default location selects the defaults. A missing explicitly selected file, unknown option or invalid value is an error. Environment overrides do not bypass parsing and validation of the file.

`user`, `backup`, `migrate` and an actual `import-legacy` run also read `KIKOERU_CONFIG`, but have no `--config` option for the new server's configuration. `db copy` reads it when selecting its default source; explicit `--from` and `--to` select the endpoints directly. `import-legacy --config` means the **old installation's `config.json`**. A read-only `import-legacy --dry-run` needs no target data directory.

The NixOS CLI wrapper already selects the service configuration. For Docker, use the `KIKOERU_CONFIG` setting in the [installation guide](install-docker.md). Changing `--data-dir` alone does not override a PostgreSQL URL. Use an [isolated configuration](backup-restore.md#test-a-restore-in-isolation) for restored copies.

## Start with a small configuration

```toml
listen = "127.0.0.1:8888"

[cache]
max_bytes = "20GB"
min_free_bytes = "5GB"
```

Save the file, then run as the data directory's owner. For example, if the file is `/etc/kikoeru-neo/config.toml` and this account can write `/srv/kikoeru-neo`:

```sh
KIKOERU_CONFIG=/etc/kikoeru-neo/config.toml \
  kikoeru-neo --data-dir /srv/kikoeru-neo serve
```

The log identifies the loaded file and listener. Opening that address in a browser should show the login page. Add [libraries](libraries.md) later in Settings. `public_url` currently has no functional consumer and is optional. It does not configure DNS, certificates, port access or a reverse proxy.

## Server options

These are server defaults. The [desktop application](desktop.md) has its own listener, database and logging rules.

| Option | Default | Purpose |
|---|---|---|
| `listen` | `"127.0.0.1:8888"` | IP address and port to listen on. Hostnames are not accepted. |
| `public_url` | Unset | An HTTP or HTTPS URL. It is not currently used to generate links. |
| `database.url` | Unset | Selects PostgreSQL. Accepts `postgres://` and `postgresql://`. |
| `trusted_proxies` | `[]` | Individual proxy IPs trusted to supply the client address for login rate limiting. |
| `roots` | `[]` | Declarative [libraries](libraries.md#declare-libraries-in-a-file). |
| `media.heavy_jobs` | `1` | Number of concurrent transcodes. |
| `media.light_jobs` | `4` | Concurrency for background probing, loudness analysis and other light jobs. On-demand playback probing has a separate pool of the same size. |
| `media.job_timeout` | `"2h"` | Timeout for a single transcode. |
| `media.queue_limit` | `32` | Maximum queued tasks, counted separately for transcodes and for loudness and waveform analysis. Further requests of that kind receive `queue_full`. |
| `media.ffmpeg`, `media.ffprobe` | Unset | Absolute executable paths. Otherwise resolved through `PATH`. The Docker image and the Nix package supply both tools; the [standalone binary](install-binary.md#install-ffmpeg-first) needs them installed. |
| `media.video_transcode` | `true` | Allows video re-encoding. Does not disable direct playback or remuxing. |
| `cache.max_bytes` | `"20GB"` | Eviction target for `cache/transcode/`, evicting least-recently-accessed entries first. Entries in use can keep usage above it, and temporary files and thumbnails are not counted. |
| `cache.min_free_bytes` | `"5GB"` | Refuses new generation jobs when free disk space falls below this threshold. |
| `log.level` | `"info"` | One of `trace`, `debug`, `info`, `warn`, `error`. |
| `metadata.source` | `"dlsite"` | `"none"` disables network metadata retrieval; new works keep their folder names. |
| `metadata.fallback` | `"none"` | `"asmr_one"` enables a fallback for RJ works when DLsite retrieval fails. |
| `metadata.hvdb` | `false` | Allows hvdb to supplement missing voice-actor information for RJ works. |
| `metadata.dlsite_url` | `"https://www.dlsite.com/"` | DLsite endpoint. Usually left unchanged. |
| `metadata.asmr_one_urls` | See below | Ordered list of fallback mirrors. Must not be empty. |
| `metadata.hvdb_url` | `"https://hvdb.me/"` | hvdb endpoint. Usually left unchanged. |

The default asmr.one mirror order is `https://api.asmr.one/`, `https://api.asmr-200.com/`, `https://api.asmr-100.com/`, then `https://api.asmr-300.com/`. With `metadata.source = "none"`, neither the fallback nor hvdb is used. These are configured endpoints, not guarantees that third-party services remain available.

Sizes accept nonnegative integer byte counts or strings such as `"512MiB"`. `KB`, `MB` and `GB` use powers of 1000; `KiB`, `MiB` and `GiB` use powers of 1024. Durations use strings such as `"90m"` or `"2h"`. Job concurrency and queue limits must be positive integers, and the job timeout must be nonzero.

## Interface settings versus file settings

Libraries added through the interface are stored in the database. Changes take effect immediately and do not rewrite TOML. Remote passwords are stored in `secrets/roots/<id>.password`. Libraries declared in the file are read-only in the interface.

The administrator's Network proxy setting (“网络代理”) is also stored in the database and takes effect immediately. It is not a TOML option. It applies to DLsite, asmr.one and hvdb requests, not OpenList or WebDAV media. The proxy must be reachable from the backend machine, or from your computer in desktop local mode. See [Security and networking](security.md) for credential and transport limits.

## NixOS module options

These options are under `services.kikoeru-neo`:

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | Enables the system service. |
| `package` | The flake's default server package | Its wrapper supplies ffmpeg and ffprobe. |
| `dataDir` | `/var/lib/kikoeru-neo` | The module creates this directory with mode `0700`. |
| `user`, `group` | `kikoeru-neo` | The module creates the default account and group. Declare custom ones yourself. |
| `database.createLocally` | `false` | Creates a local PostgreSQL database and role using peer authentication. |
| `settings` | `{}` | Generates TOML in the Nix store. Do not put passwords here. |

## Change databases

SQLite is the default. To use PostgreSQL, prepare a dedicated database owned by the service role. Supply passwords through `PGPASSWORD`, a permission-restricted `.pgpass`, or a file selected by `PGPASSFILE`, not a command-line argument. Remote database TLS parameters can go in the URL: `sslmode=verify-full` verifies certificate identity, and `sslrootcert=` selects a private CA. `sslmode=require` encrypts without verifying certificate identity.

1. [Back up](backup-restore.md) with the current version, check that no scans are running or queued, then stop the service.
2. Run `db copy` using a server package with PostgreSQL support. The source schema must match the program version. A target SQLite file must not exist; a target PostgreSQL database must be empty or contain only the application's empty tables.
3. Check the reported per-table row counts. Only after a successful copy should you change the service's database configuration and start it.

For example, with service user and PostgreSQL role `kikoeru-neo`, a pre-created target database `neo_new`, and local socket authentication on the default port:

```sh
sudo systemctl stop kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo db copy \
  --from /var/lib/kikoeru-neo/neo.sqlite3 \
  --to 'postgres:///neo_new?host=/run/postgresql&user=kikoeru-neo'
```

Set `database.url` to that target URL. On NixOS, set `settings.database.url` and reactivate the configuration. Do not start the target service before copying: startup may create records for configured libraries. If the copy refuses a target, check what it contains rather than deleting it automatically.

The copy moves only the database. Keep `secrets/`, `covers/` and `lyrics/` too. The source is not modified, but writes after the snapshot begins are not included, which is why the service must be stopped. Full legacy imports and `migrate` produce SQLite only; use `db copy` afterwards to move to PostgreSQL. `import-legacy --merge` can merge directly into PostgreSQL.

## Logs and persistent files

The server logs to stderr, collected by systemd or Docker. Routine media reads are logged at DEBUG. To inspect them temporarily, set `RUST_LOG=info,neo_server::http::access=debug`. Logs may contain usernames, paths and error details; remove private information before sharing them.

The data directory contains the database, `secrets/`, `covers/`, `lyrics/` and, when present, `import-report.json`. `cache/transcode/`, `cache/thumbs/` and `cache/tmp/` can be regenerated. Application backups exclude configuration files, external password files and original media. See the full [backup scope](backup-restore.md#what-is-included).
