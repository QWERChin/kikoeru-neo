# Try kikoeru-neo

Start the published Docker image, create an administrator and scan a library in a few minutes. You need Docker Engine 25 or newer on an `x86_64` Linux machine and a folder of media you are allowed to read. The image already contains ffmpeg and ffprobe. For a persistent installation, continue with [Docker Compose](install-docker.md) afterwards.

Work folder names must contain `RJ`, `BJ` or `VJ` followed by 6 to 8 digits, such as `RJ01234567 Example work`. This is a synthetic example, not a claim that the code exists on DLsite. See [folder naming](libraries.md#prepare-work-folders) for the full rules.

## Create an account and start

Pick a working directory and an empty configuration file. Libraries are added in the interface later:

```sh
mkdir -p neo-trial
cd neo-trial
printf '# Add libraries in Settings after signing in.\n' > config.toml
chmod 0644 config.toml
```

Create the administrator in a new volume, then start the server with your media folder mounted read-only. Replace `/srv/media/works` with your folder:

```sh
image=ghcr.io/qwerchin/kikoeru-neo:0.1.0
docker run --rm -it -e KIKOERU_CONFIG=/config/config.toml \
  -v neo-trial-data:/data -v "$PWD/config.toml:/config/config.toml:ro" \
  "$image" user create --admin admin
docker run -d --name neo-trial -p 127.0.0.1:8888:8888 \
  -v neo-trial-data:/data -v "$PWD/config.toml:/config/config.toml:ro" \
  -v /srv/media/works:/media/works:ro \
  "$image"
```

Enter a password of at least 8 characters twice. Account creation prints `created admin admin`. The container runs as uid 1000, so that user must be able to read the media folder. Run `docker ps` until the container shows `healthy`.

On an `aarch64` machine, build the image yourself as described in [Docker Compose](install-docker.md#build-with-the-dockerfile) and set `image` to the tag you built.

## Scan and play

1. Open `http://127.0.0.1:8888` in a browser on the same machine and sign in.
2. Open Settings (“设置”), select Add library (“添加媒体库”) and choose the local storage type. Enter `/media/works`, the path inside the container, not the host path.
3. Save the library and select Scan (“扫描”) beside it. Only one scan runs at a time. Leaving Settings does not cancel it.
4. Open the catalog (“资料库”), choose a work and play a track. Check audio output and seeking, then reload the page to check that progress was saved.

Duration probing for local audio and video runs in a separate queue and may finish after the scan. If metadata retrieval fails, the work still appears under its folder name and remains playable. If no works appear, check [library requirements](libraries.md) before changing anything else.

## Stop, return later or remove

```sh
docker stop neo-trial
docker start neo-trial
docker logs --tail 100 neo-trial
```

The volume `neo-trial-data` holds accounts, covers and progress, and survives `docker stop`. To remove the trial completely, run `docker rm -fv neo-trial` and `docker volume rm neo-trial-data`.

To connect from another device, configure the listener, firewall and HTTPS as described in [Security and networking](security.md). For an installation you keep, use [Docker Compose](install-docker.md), which adds a backup volume, a read-only root filesystem and restart handling.

## Without Docker

- The [standalone binary](install-binary.md) is one file for `x86_64` Linux. It needs ffmpeg and ffprobe from your distribution.
- With Nix, `nix build github:QWERChin/kikoeru-neo/v0.1.0#neo-server` produces `result/bin/kikoeru-neo`, which brings its own ffmpeg and runs like the standalone binary. The [NixOS guide](install-nixos.md) sets up a system service.
