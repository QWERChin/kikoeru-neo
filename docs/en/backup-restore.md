# Back up and restore

Use `kikoeru-neo backup` to save the database and application state. Restore into an empty directory with a matching software version. Backups contain remote credentials and user state, so restrict access to the people managing them. Back up original media, external configuration and external password files separately.

## What is included

| Content | Included by the backup command? |
|---|---|
| Database | Yes. Both SQLite and PostgreSQL are exported as `neo.sqlite3`. |
| `secrets/` | Yes, including the signing key and interface-managed OpenList and WebDAV passwords. |
| `covers/`, `lyrics/` | Yes, including covers and user-saved subtitles. |
| `import-report.json` | Yes, if present. |
| `cache/` | No. It can be regenerated. |
| `config.toml`, NixOS configuration, Compose files, external `password_file` files | No. Save these separately and protect any credentials they contain. |
| Original local or remote media | No. Use the media storage's own backup process. |

The command skips symbolic links rather than copying files outside the backup tree through them. Check the reported skip count; skipped state is not backed up.

## Create a backup

The service may remain running. The destination must not exist, its parent must exist and be writable by the command's user, and it must be outside the data directory. Choose a new name each time; `before-upgrade` below can only be used once at that location.

For a default NixOS installation:

```sh
sudo install -d -o kikoeru-neo -g kikoeru-neo -m 0700 /var/backup/kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo backup /var/backup/kikoeru-neo/before-upgrade
```

The Docker guide provides a separate backup volume. From the Compose directory:

```sh
docker compose exec -T kikoeru-neo kikoeru-neo backup /backup/before-upgrade
umask 077
mkdir backup-export
docker compose cp kikoeru-neo:/backup/before-upgrade ./backup-export/
```

Use a new `backup-export` directory so the umask restricts host access. Check files and ownership after copying, then save the backup to separate storage. Container health is not proof of backup success; check the backup command's exit status and output files. With a standalone binary, explicitly select the service's [configuration and data directory](configuration.md).

SQLite uses `VACUUM INTO` to create a consistent database snapshot including data still in the WAL. Copying only a live `neo.sqlite3` file is not equivalent. PostgreSQL exports to SQLite from a read-only snapshot transaction and requires a source schema matching the program version; no `pg_dump` is needed. The SQLite backup command does not run migrations.

## Restore to SQLite

1. Stop the service, or quit the desktop application through its tray menu. Preserve the failed installation and restore to another empty directory, not over an old directory with WAL files.
2. Copy `neo.sqlite3`, `secrets/`, `covers/`, `lyrics/` and, if present, `import-report.json` from the backup. Restoring only the database can leave keys, passwords and covers out of step.
3. Set ownership to the service account, directory modes to `0700` and file modes to `0600`. In Docker, arrange ownership for container uid 1000, accounting for rootless UID mapping.
4. Restore the associated external configuration and password files. Point the service at the restored directory and remove any configuration or environment override selecting another PostgreSQL database.
5. Start with the software version used for the backup. Check login, covers, favorites and progress. Perform an [upgrade](upgrade.md) separately if needed.

Restoring returns state to the backup time; later progress and favorites are not included. Signing keys and user token versions also return to that time. Reset passwords or sign out all devices afterwards as appropriate.

## Restore to PostgreSQL

Prepare a separate empty target database and stop the application instance that will use it. With a server package matching the backup schema, role `kikoeru-neo` and target database `neo_restored`:

```sh
sudo -u kikoeru-neo kikoeru-neo db copy \
  --from /var/backup/kikoeru-neo/before-upgrade/neo.sqlite3 \
  --to 'postgres:///neo_restored?host=/run/postgresql&user=kikoeru-neo'
```

Create the database and role with the necessary permissions beforehand. Do not use a database serving another instance. After a successful copy, restore the other state directories, set `database.url` to the target and start the service. The source backup is unchanged. See [database-copy constraints](configuration.md#change-databases).

## Test a restore in isolation

Use a separate VM or container with no original media mounts and no access to the live database, remote storage or metadata services. The restored application is not read-only. Its database may contain libraries, credentials, automatic scan times and queued tasks, so it may scan immediately on startup. Disabling automatic scans only in TOML does not clear database settings.

Inside that isolated environment, copy the backup to `/srv/neo-restore` and save this independent configuration as `/srv/neo-restore-config.toml`:

```toml
listen = "127.0.0.1:8899"

[metadata]
source = "none"
```

Run as the owner of the copy:

```sh
env -u KIKOERU_DATABASE_URL KIKOERU_LISTEN=127.0.0.1:8899 \
  kikoeru-neo --data-dir /srv/neo-restore serve --config /srv/neo-restore-config.toml
```

Explicit `serve --config` overrides the NixOS wrapper's default file. Unsetting the database environment variable prevents it from selecting PostgreSQL. For `user` or `backup`, which have no such `--config` option, explicitly set `KIKOERU_CONFIG=/srv/neo-restore-config.toml` and likewise unset `KIKOERU_DATABASE_URL`.

Check readiness, accounts, favorites, progress and covers. Unavailable remote libraries are expected under network isolation. To test playback, add only synthetic media copies. Stop the instance before deleting the test copy, and retain the original backup and your verification notes.
