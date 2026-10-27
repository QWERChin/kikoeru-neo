# Run the standalone binary

Each release includes `kikoeru-neo` as a single executable for `x86_64` Linux. It contains the server, the web interface and SQLite, needs no shared libraries and runs on any distribution, including Alpine. It does not contain ffmpeg and ffprobe: install them from your distribution before you start. For `aarch64` or other targets, build from source as described in [Contributing](../../CONTRIBUTING.md#build-the-full-server-without-nix).

## Install ffmpeg first

The server runs `ffmpeg` and `ffprobe` for duration probing, transcoding, HLS video and loudness and waveform analysis. Install both from your distribution:

| Distribution | Command |
|---|---|
| Debian, Ubuntu | `sudo apt install ffmpeg` |
| Fedora | Enable [RPM Fusion](https://rpmfusion.org/Configuration), then `sudo dnf swap ffmpeg-free ffmpeg --allowerasing` |
| Arch Linux | `sudo pacman -S ffmpeg` |
| Alpine | `sudo apk add ffmpeg` |
| openSUSE | Install `ffmpeg` from the Packman repository |

Fedora's default `ffmpeg-free` and openSUSE's default package do not include the H.264 encoder `libx264`, which video transcoding needs. Check the encoders after installing:

```sh
ffmpeg -hide_banner -encoders | grep -E ' (libx264|aac) '
ffprobe -version | head -n 1
```

Both `libx264` and `aac` must appear. If the tools are not on `PATH`, set `media.ffmpeg` and `media.ffprobe` in the [configuration](configuration.md#server-options) to their absolute paths.

Without ffmpeg the server still starts and plays formats the browser supports directly, but duration probing, transcoding, video playback through HLS, and loudness and waveform analysis fail. When `serve` starts, it checks both tools in the background and logs one warning per problem, for example:

```text
media tool check: ffmpeg "ffmpeg": missing encoder libx264; affected features: HLS video transcoding; install a complete ffmpeg build or set media.ffmpeg in config.toml
```

The server keeps running after such a warning. Install a complete ffmpeg build or set the path, then restart it. No warning is logged when both tools are complete.

## Download and verify

```sh
version=0.1.1
asset="kikoeru-neo-$version-x86_64-linux-musl"
base="https://github.com/QWERChin/kikoeru-neo/releases/download/v$version"
curl -fLO "$base/$asset"
curl -fLO "$base/SHA256SUMS"
sha256sum --check SHA256SUMS
sudo install -m 0755 "$asset" /usr/local/bin/kikoeru-neo
kikoeru-neo --version
```

`sha256sum --check` must print `OK`. Do not run a file whose checksum does not match.

## Try it as your own user

Create a separate data directory and configuration. Run this trial as your normal user, not root:

```sh
umask 077
mkdir -p demo-data
printf 'listen = "127.0.0.1:8888"\n' > demo-data/config.toml
export KIKOERU_DATA_DIR="$PWD/demo-data"
export KIKOERU_CONFIG="$PWD/demo-data/config.toml"
unset KIKOERU_DATABASE_URL KIKOERU_LISTEN
kikoeru-neo user create --admin admin
kikoeru-neo serve
```

Enter a password of at least 8 characters twice. Open `http://127.0.0.1:8888`, sign in and add a local library in Settings (“设置”) with a path your user can read, such as `/srv/media/works`. The [quickstart](quickstart.md#scan-and-play) describes the first scan and playback. Press Ctrl-C to stop the server. Keep `demo-data`: it holds accounts, covers and progress.

## Run as a system service

The repository includes a systemd unit, `packaging/systemd/kikoeru-neo.service`, which is tested in a virtual machine together with the commands below. It runs the server as the system user `kikoeru-neo`, reads `/etc/kikoeru-neo/config.toml`, keeps data in `/var/lib/kikoeru-neo` (created by systemd with mode `0700`) and finds ffmpeg in `/usr/bin` through the default `PATH`. The service may write only to its data directory. It can read local media anywhere, including under `/home`, as long as the `kikoeru-neo` user has read permission.

Create the user and the configuration, then install and start the unit:

```sh
sudo groupadd --system kikoeru-neo
sudo useradd --system --gid kikoeru-neo \
  --home-dir /var/lib/kikoeru-neo --no-create-home \
  --shell /usr/sbin/nologin kikoeru-neo
sudo install -d -o root -g kikoeru-neo -m 0750 /etc/kikoeru-neo
printf 'listen = "127.0.0.1:8888"\n' |
  sudo tee /etc/kikoeru-neo/config.toml >/dev/null
sudo chown root:kikoeru-neo /etc/kikoeru-neo/config.toml
sudo chmod 0640 /etc/kikoeru-neo/config.toml
curl -fLO https://raw.githubusercontent.com/QWERChin/kikoeru-neo/v0.1.1/packaging/systemd/kikoeru-neo.service
sudo install -m 0644 kikoeru-neo.service /etc/systemd/system/kikoeru-neo.service
sudo systemctl daemon-reload
sudo systemctl enable --now kikoeru-neo.service
curl --fail --retry 30 --retry-delay 1 --retry-connrefused \
  http://127.0.0.1:8888/api/ready
```

Wait for `/api/ready` before the next step: the first start creates the database. Management commands such as `user` and `backup` must run as the service user with the same configuration and a private umask, or the files they create get the wrong owner or mode. Define this shell function in the administrator's shell, or add it to that user's `~/.bashrc`:

```sh
kikoeru_neo() {
  sudo runuser -u kikoeru-neo -- env -i \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/var/lib/kikoeru-neo \
    KIKOERU_DATA_DIR=/var/lib/kikoeru-neo \
    KIKOERU_CONFIG=/etc/kikoeru-neo/config.toml \
    /bin/sh -c 'umask 077; exec /usr/local/bin/kikoeru-neo "$@"' sh "$@"
}
kikoeru_neo user create --admin admin
```

`env -i` keeps variables such as `KIKOERU_DATABASE_URL` from your own shell out of the command. Enter the password twice. Then open `http://127.0.0.1:8888` on the server, or reach it from another device through a [reverse proxy](security.md), and add libraries as in [Libraries and scanning](libraries.md).

Read the logs, including the ffmpeg check, with:

```sh
sudo journalctl -u kikoeru-neo.service -f
sudo journalctl -u kikoeru-neo.service -p warning -b
```

[Accounts](accounts.md) and [Backup and restore](backup-restore.md) use the same `kikoeru_neo` function.

## Upgrade

Read [Upgrades](upgrade.md) before replacing the binary. A new version may migrate the database on startup, and an older binary refuses a newer schema, so keep a [backup](backup-restore.md) and the old binary until the new version has run successfully.
