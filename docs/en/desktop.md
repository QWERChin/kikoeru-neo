# Use the desktop application

The desktop application gives you a separate window for an existing server or a library running on your computer. Choose a package for your system, then select a mode. It uses the same Chinese interface as the web application.

## Get the application

The repository's [Desktop workflow](https://github.com/QWERChin/kikoeru-neo/actions/workflows/desktop.yml) builds macOS and Windows packages. Open a successful run for your chosen version and select the matching download under Artifacts:

| System | Artifact name | Contents |
|---|---|---|
| Apple Silicon Mac | `kikoeru-neo-aarch64-apple-darwin` | A `.dmg` disk image containing the `.app`. |
| Windows x86_64 | `kikoeru-neo-x86_64-pc-windows-msvc` | NSIS installer. |

The workflow retains these packages for 30 days and does not automatically publish them to GitHub Releases. Artifact downloads normally require signing in to GitHub. An expired run or one that did not produce installers will not provide a usable package. This guide does not establish that artifacts for a particular version are currently available. The workflow does not build an Intel Mac installer.

On Linux, build from a source checkout on a machine with a graphical session. The Nix package supports `x86_64-linux` and `aarch64-linux`:

```sh
nix build .#desktop
./result/bin/kikoeru-neo-desktop
```

The package includes ffmpeg and the WebKitGTK media dependencies. Linux desktop validation still has gaps; a successful build alone does not establish that playback works on your system.

Installers are not signed with a developer certificate; macOS uses ad-hoc signing only. Your system may block the first launch. Check that the package came from the intended project workflow run, then follow your system's application-security prompts. Do not disable application verification globally. Automatic updates are not implemented. [Back up](backup-restore.md) and quit through the tray menu before replacing the installed application.

## Connect to a server

1. Select Connect to server (“连接服务器”) and enter your server's HTTP or HTTPS address. `https://media.example.lan` is a placeholder for your own address.
2. After the connection check, the application opens the server's page. Sign in with your server account.
3. Play a work and check pause, track changes and saved progress.

This mode loads the server's frontend without copying its database to your computer. The address, network and certificate must be accessible from this computer. Plain HTTP has the same limits as in a browser; see [Security and networking](security.md). To change an incorrect server address, choose Switch mode (“切换模式”) on the login or Settings page. You do not need to sign in first.

## Run a local library

1. Select Use a local library (“使用本机媒体库”). The application starts its embedded backend and signs in automatically; do not create an administrator yourself.
2. In Settings, add a local library and use the folder picker to choose a root containing work folders. Scan it using the [folder naming rules](libraries.md). OpenList and WebDAV libraries are also supported.
3. Open the catalog and play a work. Local directories are watched by default; scheduled scans still need to be enabled separately.

The embedded backend listens on a random `127.0.0.1` port and uses SQLite only. It ignores PostgreSQL and listener settings in the configuration. It is not a fixed-port server for other devices; install a separate server for that purpose.

To import kikoeru-express state, open Settings → Desktop application (“设置 → 桌面应用”) in local mode and select the old data directory. Read the preflight report, then select old users to merge into the local account and any local libraries to add. The graphical importer merges favorites, reviews and progress only. Scans still create the works. Add OpenList libraries yourself; see [Migration](migration.md) for limits.

The desktop HTTP clients do not read the macOS or Windows system proxy settings. For metadata-only proxying, enter a proxy reachable from this computer in Network proxy (“网络代理”), save it and check connectivity. That setting does not proxy media files.

## Close, quit or change mode

Closing the window hides it while playback continues. Quit (“退出”) in the tray menu stops the application and local backend. Opening the application again shows the existing window instead of starting another local instance.

Switch mode (“切换模式”) in Settings returns to mode selection. If local mode unexpectedly shows a login page, quit through the tray and reopen the application so the shell issues fresh login credentials. Do not try to guess the local account's random password.

## Data and log locations

These are the application's standard paths. Environment variables refer to the current user's system directories.

| System | Local library data | Logs |
|---|---|---|
| macOS | `$HOME/Library/Application Support/kikoeru-neo` | `$HOME/Library/Logs/io.github.qwerchin.kikoeru-neo` |
| Windows | `%APPDATA%\kikoeru-neo` | `%LOCALAPPDATA%\io.github.qwerchin.kikoeru-neo\logs` |
| Linux | `$XDG_DATA_HOME/kikoeru-neo`, or `$HOME/.local/share/kikoeru-neo` if unset or relative | `$XDG_DATA_HOME/io.github.qwerchin.kikoeru-neo/logs`, or `$HOME/.local/share/io.github.qwerchin.kikoeru-neo/logs` if unset |

The data directory has the same layout as the server's, with accounts, database, credentials, covers, subtitles and caches. Original media stays in the folders you selected. The mode and server address are stored separately in `desktop.json` under the application identifier in the system application-configuration directory.

Local-mode Settings and the startup-error page offer Open log folder (“打开日志文件夹”). Logs use UTC dates and filenames of the form `kikoeru-neo.<date>.log`. Opening the log writer keeps the newest 7 files. The default level is `info`, overridden by `RUST_LOG`; the data configuration's `log.level` does not control desktop-shell logging. A remote server page is not permitted to open the local log folder.

Before backing up, quit through the tray, then copy the complete data directory. Alternatively, use the same-version server backup command with this data directory and an explicitly selected SQLite configuration. Save `desktop.json` separately if you want to keep shell settings. Never restore over a running application. See [Backup and restore](backup-restore.md).

## Validation coverage and limitations

The repository provides these checks. They do not prove every downloaded package has been validated on your device; inspect the chosen workflow run's results.

| Area | Available checks | What they do not establish |
|---|---|---|
| macOS and Windows packages | Packaged-tool and embedded-server smoke checks using synthetic media. | The smoke check does not open the player window or prove audio output, media keys or resume from sleep. |
| Windows WebView2 | Workflow WebDriver tests for local and server modes, pages, playback and CSP. | These are not the complete Windows Rust test suite or coverage of every driver and media format. |
| Linux WebKitGTK | Nix builds and separately runnable WebDriver tests. | Linux WebDriver does not run in the Desktop workflow. |
| macOS WKWebView | Package build, smoke check and local-mode startup check. | There is no equivalent tauri-driver coverage. Video, HLS, picture-in-picture and subtitles still need physical-device validation. |

Check background playback, system media controls, sleep, saved progress when closing the window and WebView media compatibility on your target device. Windows currently skips the free-disk-space check for `cache.min_free_bytes`, though the cache size limit still applies. Canceling a Windows transcode terminates ffmpeg directly without the Unix termination-signal grace period. Local files are passed to Windows ffmpeg by path; reopening paths and special-character handling still need physical-device validation.
