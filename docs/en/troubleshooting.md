# Troubleshoot by symptom

Record the version, installation method, steps and exact error before checking service logs. [Back up](backup-restore.md) before making recovery changes. Do not delete the database, recreate volumes or clean up missing works just to diagnose a problem. Redact your paths, usernames, credentials and work information from logs and screenshots before sharing them.

## Find the logs

On NixOS:

```sh
systemctl status kikoeru-neo
journalctl -u kikoeru-neo -n 100 --no-pager
```

For Docker, from the Compose directory:

```sh
docker compose ps
docker compose logs --tail 100 kikoeru-neo
```

A standalone server writes to terminal stderr. Desktop local mode offers Open log folder (“打开日志文件夹”) in Settings or on the startup-error page; see [desktop paths](desktop.md#data-and-log-locations). Task-completion logs include task IDs that can be matched to Settings.

## The page will not open, or the container is unhealthy

1. Confirm the process is running and check for configuration or database-migration errors.
2. Request `/api/ready` at the listener address from the server machine. At the default port, `http://127.0.0.1:8888/api/ready` should return HTTP 200.
3. If local access works but other devices fail, check the listener, firewall, Docker port mapping and reverse proxy. A loopback listener does not accept direct connections from other machines, and `public_url` does not change it.

The Docker image cannot start its default command without `/config/config.toml`. Its `KIKOERU_LISTEN` overrides the file's `listen`, and the health check uses that environment variable too. Clearing it requires a matching health-check change. The image has no `/bin/sh`; see [Docker installation](install-docker.md#listener-and-health-check) for an exec-form check.

A normal Cargo backend build does not embed the frontend by default. If the API works but the home page returns a JSON 404, use the Nix server package, which includes the frontend.

## An administrator was created, but the page still asks for initialization

The account command may have written to another database. Compare `--data-dir`, `KIKOERU_CONFIG` and `KIKOERU_DATABASE_URL` with the service's actual settings. Docker's `serve --config` is not inherited by a separate `user` command; use the shared environment setting in the installation guide. On NixOS, use the module-installed CLI as the service user.

Imported accounts have no passwords. Set them with the [account commands](accounts.md). Desktop local mode should sign in automatically; quit completely and reopen it if it shows a login page.

## Login is rate-limited even with the correct password

Failed attempts accumulate by source address, and successful login does not clear earlier failures. Once the address limit is reached, wait for the counting window to expire, which can take about 2 minutes. Users behind a proxy or NAT may share an address. Check logged client addresses and `trusted_proxies` rather than trusting all sources to bypass the limit. Username-based limiting adds delay rather than a direct ban. See [Security and networking](security.md).

## The server rejects data-directory permissions

The data directory and SQLite file must not be writable by the group or other users. `secrets/` and the signing key must also be private, and `jwt.key` must be a regular file. After a restore, use service-account ownership, `0700` directories and `0600` files.

NAS-backed Docker data volumes, `0777` mounts or permission changes from `fsGroup` can trigger these checks. Store application data on a filesystem that can retain the required permissions. Media storage is separate and can be mounted read-only.

## A library is unavailable, or a scan finds no works

- For local storage, check paths from the server or container's point of view, mount availability and the service account's ability to list directories. An empty root is also reported unavailable.
- Check that work folders have `RJ`, `BJ` or `VJ` codes, that the library selects their parent directory, and that the layout does not depend on symbolic links. See [folder rules](libraries.md#prepare-work-folders).
- For OpenList, use an administrator account with account base path `/`, a readable password file and the correct library `base_path`. Listing access alone does not grant the `fs/link` permission needed for playback.
- For WebDAV, use the actual DAV endpoint, not a browser login page. Check Basic credentials and same-origin redirects.
- A scan can show completed despite some directory-listing failures. Check task logs rather than treating completion as proof every work was imported.

If deep changes do not appear, select “完整扫描（刷新网盘缓存）” to traverse OpenList or WebDAV works again. This full-scan option does not refresh OpenList's cache. Resolve stale upstream listings in OpenList itself. If a network mount does not deliver file notifications, add scheduled scans instead of relying only on `watch`.

## Tracks exist but covers, titles or durations are missing

Metadata failures do not prevent import. Folder names and placeholder covers are valid fallback results. Check source-request errors, whether `metadata.source` is `none`, and whether the metadata proxy is reachable from the backend. The desktop application does not read system proxy settings.

A completed scan does not mean all local probing has finished. Remote audio and video are usually probed on first playback. Retrying metadata on the detail page does not rescan media files. Remote cover reads also depend on storage connectivity and rate limits.

## Playback fails, preparation takes a long time, or `queue_full` appears

Distinguish unavailable storage, unsupported browser formats, queued background tasks and insufficient disk space. Check Settings tasks and logs, and confirm ffmpeg and ffprobe can run. The Nix package and image include these tools; copying an individual binary does not carry its dependencies with it.

`queue_full` means the queue for that kind of task, transcoding or loudness and waveform analysis, is at its limit. Wait for those tasks to finish before trying again. New generation jobs are refused below `cache.min_free_bytes`; free unrelated space or adjust the cache budget without deleting the database or credentials. With `media.video_transcode` disabled, videos that need re-encoding fail rather than being transcoded.

The proxy must preserve Range requests and responses, avoid caching login responses and avoid buffering task event streams. Browser-console CSP and media errors can help separate proxy problems from player problems. For background, lock-screen or media-key failures, record the browser and operating system. Emulator and package smoke checks do not prove those behaviors; see [desktop validation limits](desktop.md#validation-coverage-and-limitations).

## PWA installation or offline opening is unavailable

Ordinary LAN HTTP is not a secure context and does not provide Service Workers. Login and online playback still work. Service-worker-based installation and offline opening require HTTPS or a browser-recognized local origin. Revisit over HTTPS, but do not interpret successful installation as support for offline media downloads. See [Security and networking](security.md).

## The service will not start after an upgrade

Keep the logs and current data. Correct invalid configuration if that is the error. If the schema is newer than the program, use matching software or [restore an older backup](backup-restore.md). Do not edit migration records to bypass version checks. Resubmit interrupted scans after the service is working.
