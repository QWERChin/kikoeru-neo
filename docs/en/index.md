# kikoeru-neo user guide

kikoeru-neo lets you browse and play your own audio dramas, audiobooks and videos, with work metadata from DLsite. This guide covers installing and running a server, managing a library and using the desktop application. The interface is currently in Chinese. Instructions include the Chinese labels where you need to find a control.

You need readable media folders or an OpenList or WebDAV service. The scanner identifies works by codes in folder names, so an ordinary folder without a work code will not become a work automatically. Start with the [quickstart](quickstart.md), then choose an installation method.

## Install and start listening

| Your task | Guide |
|---|---|
| Try the server on Linux | [Quickstart](quickstart.md) |
| Run a NixOS system service | [Install on NixOS](install-nixos.md) |
| Run with Docker Compose | [Install with Docker](install-docker.md) |
| Use a separate window or a library on your computer | [Desktop application](desktop.md) |
| Keep favorites, reviews and progress from kikoeru-express | [Migrate existing data](migration.md) |

## Manage your installation

- [Configuration](configuration.md) explains configuration files, environment variables, defaults and database selection.
- [Libraries and scanning](libraries.md) covers folder names, storage credentials, manual scans and automatic scans.
- [Accounts](accounts.md) covers creating users, resetting passwords and revoking sessions.
- [Backup and restore](backup-restore.md) lists what a backup contains and how to test a restore in isolation.
- [Upgrades](upgrade.md) covers version pinning, database migrations and returning to an older version.
- [Troubleshooting](troubleshooting.md) starts from login, scanning and playback symptoms.
- [Security and networking](security.md) explains HTTPS, reverse proxies, plain HTTP and credential handling.

In these guides, a **library** is a storage root containing work folders. A **work folder** holds one work. The **catalog** is the combined browsing view, labeled “资料库” in the interface. Accounts, work codes, paths and addresses in examples are synthetic. Replace installation-specific values with your own.

The [design book](https://github.com/QWERChin/kikoeru-neo/blob/main/book/src/SUMMARY.md) is in Chinese only and is intended for implementers. You do not need it to install or operate the application. The project is licensed under GPL-3.0-or-later.
