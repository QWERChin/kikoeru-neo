# Migrate from kikoeru-express

The importer reads SQLite data from kikoeru-express 0.6.14 through 0.6.17 without modifying the source. Use a full import for a new instance or a merge for an instance that already has scanned libraries. Stop the source service first, keep a complete backup of its database, configuration and covers, and prepare an account and enough disk space for the new instance.

## Choose an import method

| Method | Target | What it preserves |
|---|---|---|
| `import-legacy` | A nonexistent or empty data directory; produces SQLite only. | Work metadata, locations, users, favorites, reviews, old playback positions and main covers. A later scan creates tracks. |
| `import-legacy --merge` | An existing SQLite or PostgreSQL database. | Favorites, reviews and progress matched by user and work code. Does not import work metadata, covers or libraries. |
| `migrate` | Selects a full import or merge based on whether a target database exists; accepts SQLite configuration only. | A Chinese-language interactive guide using the same importer, with configuration and password setup. |
| Container migration script | A new Docker data volume. | Copies input from a stopped old container and runs `migrate` in the new image. |

MySQL sources are not supported. For earlier versions, upgrade a separate copy with a pinned version of the old application to a supported format. Do not open your only rollback copy with an unpinned old image. Imports do not copy original audio or video, or migrate lyric translation jobs. The report lists skipped and unmatched records.

## Inspect the source copy

These examples assume a backup made after stopping the old service at `/srv/kikoeru-legacy-backup`, with old libraries named `Local` and `Cloud`. These are synthetic values. Run a preflight first, then use the old names in your own report for mappings.

```sh
kikoeru-neo import-legacy --source /srv/kikoeru-legacy-backup --dry-run
```

`--source` accepts the old data directory, its `sqlite/` or `config/` directory, `db.sqlite3`, or a Docker volume directory containing `_data`. The preflight reports located inputs, missing mappings, field problems and reasons the import would be refused. `--json` requires `--dry-run`. Exit status is 0 for success, 2 when the import would be refused, and 1 for other failures.

| Option | When to use it |
|---|---|
| `--db FILE`, `--covers DIR`, `--config FILE` | Override the old database, covers directory or old `config.json` location. |
| `--root OLD=NEW` | Map an old `root_folder` name to a new library ID. Repeat as needed. |
| `--map-path OLD=NEW` | Map an old configuration path prefix to its location on the new machine or in the new container. Repeat as needed; the longest matching prefix wins. |
| `--accept-guessed` | Accept paths marked `guessed`, after checking them. |
| `--recover-journal` | After confirming the source service is stopped, allow recovery of a nonempty journal or WAL in the tool's private copy. |
| `--write-config FILE` | Write a library configuration draft to a new file. Does not overwrite an existing file. |

`--root` maps library names; `--map-path` maps filesystem paths. They are not interchangeable. Full imports require mappings for all old libraries containing works and reject misspelled old names.

## Perform a full import

Run as an account that can read the source copy and create the target. The subshell below selects an empty configuration to prevent a NixOS wrapper or inherited environment from selecting an existing PostgreSQL database. Choose unused target and output paths.

```sh
(
  set -e
  umask 077
  import_config=$(mktemp)
  trap 'rm -f "$import_config"' EXIT
  export KIKOERU_CONFIG="$import_config"
  unset KIKOERU_DATABASE_URL

  kikoeru-neo import-legacy --dry-run \
    --source /srv/kikoeru-legacy-backup \
    --root Local=local --root Cloud=cloud \
    --map-path /usr/src/kikoeru/Local=/srv/media/works
)
```

After checking that the preflight has no refusal reasons, import under the same isolated configuration:

```sh
(
  set -e
  umask 077
  import_config=$(mktemp)
  trap 'rm -f "$import_config"' EXIT
  export KIKOERU_CONFIG="$import_config"
  unset KIKOERU_DATABASE_URL

  kikoeru-neo --data-dir /srv/kikoeru-neo-imported import-legacy \
    --source /srv/kikoeru-legacy-backup \
    --root Local=local --root Cloud=cloud \
    --map-path /usr/src/kikoeru/Local=/srv/media/works \
    --write-config /srv/kikoeru-neo-imported-roots.toml
  kikoeru-neo --data-dir /srv/kikoeru-neo-imported user set-password admin
)
```

The password command runs only after a successful import. Replace `admin` with the retained administrator in your report. Old passwords are not migrated; set passwords for every account that needs to sign in.

Read `import-report.json` in the target directory and resolve the generated configuration's `# TODO` entries and commented-out libraries. Old OpenList passwords are not read. Create files referenced by `password_file`, or add interface-managed libraries with the same IDs. Keep passwords out of the Nix store.

Configure the new service to use the reviewed configuration and imported data directory. On NixOS, set `dataDir`, make the service user the owner, and use `0700` for directories and `0600` for files. Do not start a service in the target before importing or mix the import into an existing nonempty directory. Configured library IDs must match the import mappings.

For interactive setup, use this command under the same isolated configuration instead of the full-import command:

```sh
kikoeru-neo --data-dir /srv/kikoeru-neo-imported migrate --source /srv/kikoeru-legacy-backup
```

The prompts are in Chinese and ask for paths, library IDs, OpenList passwords and account passwords. A noninteractive run requires `--yes`, which accepts defaults and sets no passwords. Configuration is written to the target's `config.toml`, or to `config.from-kikoeru.toml` if `config.toml` already exists. Merge the latter into your active configuration yourself.

## Merge into an existing instance

[Back up](backup-restore.md) the target first. A merge may run while the service is running and must use its actual configuration and database. For a default NixOS installation, with a source copy readable by the service user:

```sh
sudo -u kikoeru-neo kikoeru-neo import-legacy --merge --dry-run \
  --source /srv/kikoeru-legacy-backup --user old-reader=reader
sudo -u kikoeru-neo kikoeru-neo import-legacy --merge \
  --source /srv/kikoeru-legacy-backup --user old-reader=reader
```

Read the preflight before running the second command. In Docker, mount the source copy read-only and use paths as seen inside the container.

Users are matched by name unless you supply repeatable `--user OLD=NEW` mappings, available only with `--merge`. Missing users are created as ordinary users without passwords; set them with the [account commands](accounts.md). Existing target favorites and reviews are retained. Progress uses the newer update time. State for works not yet scanned is held until the target discovers the same work code.

A merge does not create libraries. Add them through [library settings](libraries.md), or use `--write-config` for a draft. In merge mode, `--root` only chooses IDs in that draft, not how user state is matched.

## Migrate an old container

[Build and load the Docker image](install-docker.md), but do not create a target volume or start the new service yet. Prepare the stopped old container and an independent backup. Here `legacy-player` stands for your old container:

```sh
docker stop legacy-player
nix run .#kikoeru-neo-docker-migrate -- \
  --from-container legacy-player --image kikoeru-neo:0.1.0 --out ./neo
```

Run `nix run` from the source checkout and use the image tag actually loaded. The script inspects and copies the old container without starting it. When both input and output are terminals, it prompts interactively. Otherwise it uses `migrate --yes` and sets no passwords.

By default it creates `kikoeru-neo-data` and refuses an existing volume of that name. Use `--volume` to choose another name. `--out` defaults to `./neo`, which must not already contain `config.toml`. `--stage-dir` defaults to the current directory; it must exist and have room for temporary database and cover copies. Avoid a memory-backed filesystem without enough space.

If automatic discovery fails, `--data-root` specifies the old data directory inside the old container. `--root` and `--map-path` are passed to the migration guide; new paths refer to the new container's view. Set `DOCKER=podman` to use Podman.

After success:

1. Read the TODO entries in `./neo/config.toml`. Check media mounts and OpenList password-file paths.
2. Save the printed Compose fragment and add the backup volume and `KIKOERU_CONFIG=/config/config.toml` from the [Docker guide](install-docker.md). Preserve the actual configuration path and media mounts printed for your migration. The script does not write `compose.yaml` itself.
3. Set any missing account passwords, using the names in your report. For the default volume and example administrator:

   ```sh
   docker run --rm -it -v kikoeru-neo-data:/data \
     kikoeru-neo:0.1.0 user set-password admin
   ```

4. Start the new Compose service, wait for readiness, sign in and scan.

Media stored only in the old container's writable layer does not become a mount automatically. Save and mount that media separately. The script handles old configuration, database and covers only. If import fails, it attempts to remove the volume it created for this run; check the error output. It does not remove pre-existing volumes.

## Scan and verify

After a full import, scan all corresponding libraries to create tracks. An old playback position can be restored exactly only when its full relative path uniquely matches a track in the same work. Unmatched or ambiguous positions remain pending; existing progress changed in the new system is not replaced by an arbitrary old position.

Compare your preflight, import report and interface results for works, favorites, reviews and progress. Sample covers and playback. Favorites, completion markers and history can overlap, so adding old table counts does not give an expected progress total. `import-report.json` records the import and is not refreshed by later scans.

Keep the original backup until you have checked both the migration and your ability to restore. Reports contain your paths and user data; redact them before sharing. The desktop local-mode importer only merges user state, as described in [Desktop application](desktop.md).
