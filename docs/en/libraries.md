# Add libraries and scan for works

A library is a storage root containing work folders, backed by a local filesystem, OpenList or WebDAV. An administrator must add and scan it before works appear in the catalog. Prepare the folder layout and storage credentials first.

## Prepare work folders

The scanner searches down to 16 levels below a library root. A folder whose name contains `RJ`, `BJ` or `VJ` followed by 6 to 8 digits becomes a work folder. The scanner then stops looking for nested works inside it. Codes are case-insensitive, cannot immediately follow an English letter, and cannot contain only zero digits.

This synthetic layout illustrates the structure; use your own works' codes:

```text
/srv/media/works/
  RJ01234567 Example work/
    01.wav
    Bonus/
      02.mp3
```

Set the library path to `/srv/media/works`, not the individual work folder. Folders without codes are traversed as categories; audio filenames do not create works. Symbolic links are not followed. Multiple locations with the same work code appear as one work, with locations listed on its detail page.

## Add a library in the interface

Sign in as an administrator, open Settings (“设置”) and select Add library (“添加媒体库”). Enter the name, storage type and connection details. Check the generated library ID before saving: it is used in links and the database and cannot be changed afterwards. Interface-managed libraries are stored in the database and changes take effect immediately.

| Type | Requirements | Meaning of the path |
|---|---|---|
| Local | The service account can traverse directories and read files. | An absolute path on the backend machine, or a mounted path inside Docker. |
| OpenList | An OpenList administrator account whose account base path is `/`. | `base_path` is the media root within OpenList. |
| WebDAV | A server supporting `PROPFIND` and file reads, with HTTP Basic credentials. | `url` is the DAV endpoint; `base_path` is the media folder below it. |

OpenList playback uses `fs/link`, which requires administrator access. An ordinary account that can list folders is not sufficient. The account's base path `/` is separate from the kikoeru-neo library's `base_path`, which should still select the desired media folder.

WebDAV supports Basic authentication only. It follows at most 5 redirects, and only when the scheme, host and port stay unchanged. A browser login page or cross-host login redirect is not a usable DAV endpoint. See [Security and networking](security.md) for HTTP and certificate limits.

## Declare libraries in a file

To manage libraries through configuration, use `[[roots]]` in `config.toml` or the corresponding fields in NixOS `settings.roots`. File-managed libraries are read-only in the interface and require a restart after changes. Use either the interface or the file for a given ID, not both.

```toml
[[roots]]
id = "local"
display_name = "Local works"
type = "local"
path = "/srv/media/works"
watch = true
auto_scan_interval = "1d"
```

Replace the OpenList address, account and path below with your own. Store the password in a file readable by the service user, not directly in TOML:

```toml
[[roots]]
id = "cloud"
display_name = "Cloud works"
type = "openlist"
url = "http://192.0.2.10:5244"
base_path = "/media/works"
username = "library-admin"
password_file = "/run/secrets/openlist-password"
list_concurrency = 4
link_rate = 1.0
auto_scan_interval = "6h"
```

`192.0.2.10` stands for your OpenList host; it is not a reachable example service. This HTTP example is for a trusted network only. A corresponding WebDAV configuration is:

```toml
[[roots]]
id = "dav"
display_name = "WebDAV works"
type = "webdav"
url = "http://192.0.2.20:8080/dav"
base_path = "/media/works"
username = "library-reader"
password_file = "/run/secrets/webdav-password"
list_concurrency = 4
auto_scan_interval = "6h"
```

The URL path and `base_path` are combined, so do not repeat `/dav` in both. Both remote types default to 4 concurrent listings. OpenList link requests default to 1 per second. Raising these limits may trigger remote rate limits.

IDs must be nonempty and contain only ASCII letters, digits, `-` and `_`. Keep them stable when changing a display name or server address. In Docker, mount password files read-only at the paths the container configuration names.

## Run a scan and check the result

1. After saving, select Scan (“扫描”) beside the library or Scan all (“扫描全部”). Check that storage is reachable and that the task moves from queued to running.
2. Wait for work discovery, track synchronization, metadata and cover retrieval. Only one scan runs at a time across all libraries. Submitting the same library again reuses its queued or running task.
3. Open the catalog and a work's detail page. Check locations, tracks, covers and playback. Local duration probing may still be running; remote audio and video are usually probed when first played.

Leaving the page stops displaying progress but does not stop the task. Use Cancel task (“取消任务”) to cancel. A restart marks running scans as interrupted; submit a new scan to continue the work. Queued tasks are restored to the queue.

The full-scan control is currently labeled **“完整扫描（刷新网盘缓存）”**, meaning “Full scan (refresh cloud-drive cache).” Despite that label, it only disables the work-directory listing-hash shortcut for OpenList and WebDAV, making the scanner traverse each work again. **It does not refresh OpenList's cache.** Use it when deep changes were not reflected in a parent listing. If OpenList itself still returns stale listings, handle its cache in OpenList first. Automatic scans are ordinary scans, not full scans.

Metadata failures do not prevent importing or playing a work. It can initially use its folder name and a placeholder cover, or a cover found in its folder. Retry metadata retrieval from the detail page when connectivity returns. Scanning reads candidate cover images as well as directory listings.

## Automatic scans and filesystem watching

The server enables neither automatic scans nor local filesystem watching by default. Desktop local mode watches local folders by default. Change these settings in the library editor or through `auto_scan_interval` and `watch` in the file.

| Setting | Range and behavior |
|---|---|
| Local `auto_scan_interval` | `5m` to `30d`; omitted means disabled. |
| OpenList or WebDAV `auto_scan_interval` | `30m` to `30d`; omitted means disabled. |
| Local `watch` | Queues a scan after changes have been quiet for 10 seconds. |
| Remote `watch` | Unsupported; cannot be `true`. |

The interval starts at the end of the last scan, including failed or canceled scans. A never-scanned library counts from when automatic scanning is enabled, rather than scanning immediately on creation. A restored instance can already be overdue when it starts.

Network shares and Docker Desktop mounts may not deliver filesystem notifications. A watching status does not prove every change reaches the watcher. Add scheduled scans for these libraries and check that a newly added file appears.

## Missing content and other limits

A failed directory listing, such as disconnected storage or denied access, does not mark existing works missing. An empty local root is reported as unavailable; add the first work before scanning. A successful but empty OpenList or WebDAV listing is different: it can mark every location in that library missing, so after the remote folder was emptied or moved, check that it lists your works before scanning. If some work folders cannot be traversed, existing tracks are retained, but the overall task can still show completed. Detailed scan summaries are not currently exposed by the task API, so check logs when content is missing.

Works confirmed missing are hidden by default while their favorites and progress remain. An administrator can delete their database records under Missing works and locations (“缺失的作品和位置”). Removing a work also removes its favorites, progress and covers, so back up first. Deleting a library does not delete source media. Re-adding the same ID and path and scanning can restore content that still exists.
