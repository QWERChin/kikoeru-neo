# kikoeru-neo

A self-hosted player for audio dramas and audiobooks, with metadata from DLsite. Browse and play from phone or desktop browsers, or use the desktop app. This is an independent rewrite that reads the old kikoeru-express data format.

The interface is currently in Simplified Chinese. The screenshots use synthetic media.

![Library in a desktop browser](docs/screenshots/desktop-library.webp)

<table>
  <tr>
    <td><img src="docs/screenshots/phone-library.webp" alt="Library on a phone"></td>
    <td><img src="docs/screenshots/phone-detail-playing.webp" alt="Work details during playback"></td>
    <td><img src="docs/screenshots/phone-player-dark.webp" alt="Full-screen player in dark mode"></td>
  </tr>
</table>

<details>
<summary>More screenshots</summary>

![Full-screen player with subtitles](docs/screenshots/desktop-player.webp)
![Video with ASS subtitles](docs/screenshots/desktop-video.webp)
![Settings while scans are queued](docs/screenshots/desktop-settings.webp)

</details>

## Features

- Browse covers, search by circle, voice actor or tag, and keep favorites and reviews.
- Resume playback across devices with server-saved progress. Background playback and media controls depend on the browser and operating system.
- Read local folders, OpenList and WebDAV libraries, with scheduled scans and optional local file watching.
- Fetch metadata and covers from DLsite while preserving manual edits during normal refreshes.
- Play audio with transcoding when needed, and video with HLS fallback and ASS subtitles. Audio also supports waveforms and loudness normalization.
- Use a browser, install a PWA in a supported secure context, or connect through the desktop app.

## Quick start

On `x86_64-linux` or `aarch64-linux`, install Nix with flakes enabled. In a fresh working directory, build the server and create a trial configuration:

```sh
nix build github:QWERChin/kikoeru-neo#neo-server
umask 077
mkdir demo-data
printf 'listen = "127.0.0.1:8888"\n' > demo-data/config.toml
export KIKOERU_DATA_DIR="$PWD/demo-data"
export KIKOERU_CONFIG="$PWD/demo-data/config.toml"
unset KIKOERU_DATABASE_URL KIKOERU_LISTEN
./result/bin/kikoeru-neo user create --admin admin
./result/bin/kikoeru-neo serve
```

Enter a password of at least 8 characters when prompted. Open `http://127.0.0.1:8888`, sign in, and add a local library in Settings. For example, use ID `local`, name `Local works` and a readable server path such as `/srv/media/works`. Work folder names must contain `RJ`, `BJ` or `VJ` followed by 6 to 8 digits, such as the synthetic `RJ01234567 Example work`.

Save the library, start its scan, then open a work and play a track. Press Ctrl-C to stop the server. Keep `demo-data` to retain accounts and progress. See the [quick-start guide](docs/en/quickstart.md) for returning later and the [security guide](docs/en/security.md) before exposing the service to another device. Core playback supports HTTP; Service Workers and PWA installation require a supported secure context.

## Guides

- [User guide](docs/en/index.md), including [NixOS installation](docs/en/install-nixos.md) and [Docker installation](docs/en/install-docker.md).
- [Configuration](docs/en/configuration.md), [libraries and scans](docs/en/libraries.md), and [accounts](docs/en/accounts.md).
- [Migration from kikoeru-express](docs/en/migration.md), [backup and restore](docs/en/backup-restore.md), and [upgrades](docs/en/upgrade.md).
- [Desktop app](docs/en/desktop.md) and [troubleshooting](docs/en/troubleshooting.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for development, tests and pull requests. The implementation design book is maintained in Chinese only; it is not required for installing or using the player.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
