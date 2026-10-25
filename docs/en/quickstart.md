# Try kikoeru-neo on Linux

Build the server with Nix, create an administrator and scan a library without setting up a system service. The Nix server package supports `x86_64-linux` and `aarch64-linux`. For a persistent installation, use the [NixOS](install-nixos.md) or [Docker](install-docker.md) guide.

## Before you start

Install Git and Nix with flakes enabled. Prepare media you are allowed to read. Work folder names must contain `RJ`, `BJ` or `VJ` followed by 6 to 8 digits, such as `RJ01234567 Example work`. This is a synthetic example, not a claim that the code exists on DLsite. See [folder naming](libraries.md#prepare-work-folders) for the full rules.

## Build and create an account

```sh
git clone https://github.com/QWERChin/kikoeru-neo.git
cd kikoeru-neo
nix build .#neo-server
```

In the same terminal, create a separate configuration. Run this trial as your normal user, not root:

```sh
umask 077
mkdir -p demo-data
printf 'listen = "127.0.0.1:8888"\n' > demo-data/config.toml
export KIKOERU_DATA_DIR="$PWD/demo-data"
export KIKOERU_CONFIG="$PWD/demo-data/config.toml"
unset KIKOERU_DATABASE_URL KIKOERU_LISTEN
./result/bin/kikoeru-neo user create --admin admin
./result/bin/kikoeru-neo serve
```

Enter a password of at least 8 characters twice. Account creation prints `created admin admin` on success. The server runs in the foreground and writes logs to the terminal. The package supplies ffmpeg and ffprobe.

## Scan and play

1. Open `http://127.0.0.1:8888` in a browser on the same machine and sign in.
2. Open Settings (“设置”), select Add library (“添加媒体库”) and choose the local storage type. Enter an absolute path on the server, such as `/srv/media/works`. The user running the server must be able to list the directories and read the files.
3. Save the library and select Scan (“扫描”) beside it. Only one scan runs at a time. Leaving Settings does not cancel it.
4. Open the catalog (“资料库”), choose a work and play a track. Check audio output and seeking, then reload the page to check that progress was saved.

Duration probing for local audio and video runs in a separate queue and may finish after the scan. If metadata retrieval fails, the work still appears under its folder name and remains playable. If no works appear, check [library requirements](libraries.md) before changing the data directory.

## Stop and return later

Press Ctrl-C in the server terminal to stop it. To start it again from the same checkout:

```sh
KIKOERU_CONFIG="$PWD/demo-data/config.toml" \
  ./result/bin/kikoeru-neo --data-dir "$PWD/demo-data" serve
```

Keep `demo-data`: it holds accounts, covers and progress. The exported environment variables last only for that terminal session. To connect from another device, configure the listener, firewall and HTTPS as described in [Security and networking](security.md).

This trial uses the source you cloned. For a persistent installation, [pin the version and keep backups](upgrade.md).
