# Run with Docker Compose

Run kikoeru-neo as a long-lived container with persistent storage, backups and an administrator. You need Docker Engine 25 or newer and Docker Compose v2 on Linux. The image contains the server, the web interface, ffmpeg and ffprobe, so nothing else has to be installed on the host. Docker Desktop on macOS and Windows has not been tested.

## Get the image

Choose one of the three ways below. All of them produce the same image layout, and the rest of this page works the same for each.

### Pull the published image

Release images are published for `x86_64` Linux:

```sh
docker pull ghcr.io/qwerchin/kikoeru-neo:0.1.0
```

`0.1.0` is a fixed version. `0.1` follows the newest patch release of the 0.1 series. No `latest` tag is published, so an upgrade only happens when you change the tag. The image is not published for `aarch64`; build it with the Dockerfile instead.

### Build with the Dockerfile

This needs only Git and Docker, on `x86_64` or `aarch64` Linux. The Dockerfile runs Nix inside the build container and produces the same build that CI tests:

```sh
git clone --branch v0.1.0 https://github.com/QWERChin/kikoeru-neo.git
cd kikoeru-neo
docker build -t kikoeru-neo:0.1.0 .
```

The first build compiles the Rust server and a reduced ffmpeg from source, which took about 10 minutes on a 24-core test machine. The build cache needs about 10 GB of disk; `docker builder prune` removes it afterwards, and the final image is about 175 MB. Later builds reuse Docker's layer cache only when nothing in the source changed. A Dockerfile build has no Git metadata, so `/api/version` reports the version but no commit. Set `image: kikoeru-neo:0.1.0` in the Compose file below.

### Build with Nix

With Nix and flakes enabled on Linux, the image is built without Docker and loaded afterwards:

```sh
nix build github:QWERChin/kikoeru-neo/v0.1.0#docker-image --out-link docker-image
./docker-image | docker load
```

`docker-image` is a script that streams the image, not a tar file. It loads as `kikoeru-neo:0.1.0`. Set that name as `image` in the Compose file below.

## Prepare Compose

Create a deployment directory for `compose.yaml` and `config.toml`. Start with an empty configuration and add libraries through the interface:

```sh
mkdir -p neo-deploy
cd neo-deploy
printf '# Add libraries in Settings after signing in.\n' > config.toml
chmod 0644 config.toml
```

Save this as `compose.yaml`:

```yaml
services:
  kikoeru-neo:
    image: ghcr.io/qwerchin/kikoeru-neo:0.1.0
    restart: unless-stopped
    stop_grace_period: 30s
    read_only: true
    tmpfs: [/tmp]
    cap_drop: [ALL]
    security_opt: ["no-new-privileges:true"]
    ports: ["127.0.0.1:8888:8888"]
    environment:
      KIKOERU_CONFIG: /config/config.toml
    networks: [neo]
    volumes:
      - data:/data
      - backup:/backup
      - type: bind
        source: ./config.toml
        target: /config/config.toml
        read_only: true
        bind: { create_host_path: false }
networks:
  neo:
    ipam:
      config:
        - subnet: 172.30.250.0/24
          gateway: 172.30.250.1
volumes:
  data: { name: kikoeru-neo-data, external: true }
  backup: { name: kikoeru-neo-backup }
```

`172.30.250.0/24` is an example private subnet. Choose a different private range if it overlaps your networks. Defining a gateway does not make it a trusted proxy; verify the actual proxy source before configuring `trusted_proxies`.

`KIKOERU_CONFIG` makes `user`, `backup` and `serve` read the same configuration. The image's default command passes `/config/config.toml` only to `serve`, not to other subcommands.

For local media, add this read-only bind mount under the service's `volumes`. First create the host directory and make sure container uid 1000 can read it:

```yaml
      - type: bind
        source: /srv/media/works
        target: /media/works
        read_only: true
        bind: { create_host_path: false }
```

Enter `/media/works` in the interface, not the host path. No media mount is needed for OpenList or WebDAV alone.

## Create an administrator and start

If you are migrating an old container, skip this section and follow [container migration](migration.md#migrate-an-old-container). That script creates its own empty target volume.

For a new installation:

```sh
docker volume create kikoeru-neo-data
docker compose run --rm kikoeru-neo user create --admin admin
docker compose up -d
docker compose ps
docker compose logs --tail 100 kikoeru-neo
```

Enter the password when prompted. Account creation should print `created admin admin`. Wait for the service to become `healthy`, then open `http://127.0.0.1:8888` on the Docker host. For a remote host, use [SSH port forwarding](install-nixos.md#activate-create-an-administrator-and-connect) or configure a [reverse proxy](security.md). Sign in and follow [Libraries and scanning](libraries.md) to reach first playback.

## Everyday administration

Run `docker compose logs -f kikoeru-neo` from the deployment directory to follow logs. See [Accounts](accounts.md), [Backup and restore](backup-restore.md) and [Upgrades](upgrade.md) for routine commands. Keep the data volume. `docker compose down -v` can delete non-external volumes, including this example's backup volume.

The container runs as `1000:1000`. Fresh named volumes inherit the image's `0700` permissions on `/data` and `/backup`. If you use bind mounts instead, arrange read/write access for uid 1000 inside the container. Rootless Docker maps host UIDs differently, so host-side `chown 1000` is not a universal solution. The data directory and SQLite file must not be group-writable. A NAS data mount with `0777` permissions is unsuitable; media mounts only need read access.

To use PostgreSQL, set `database.url` in `config.toml` or override it with `KIKOERU_DATABASE_URL`. All subcommands will see it because the Compose file sets `KIKOERU_CONFIG`. Keep passwords out of the image and shared configuration examples. See [Configuration](configuration.md).

## Listener and health check

The image sets `KIKOERU_LISTEN=0.0.0.0:8888`, overriding `listen` in the configuration file. Usually you only need to change the host port in `ports`. To change the container port, update both `KIKOERU_LISTEN` and the mapping. The health check follows that environment variable.

The health check requests `/api/ready` every 30 seconds, has a 10-second timeout and marks the container unhealthy after 3 consecutive failures. The initial grace period is 120 seconds. If you clear `KIKOERU_LISTEN` to use the file's listener, give the health check its own address. For a file setting of `listen = "0.0.0.0:9000"`, replace the relevant Compose entries with:

```yaml
    environment:
      KIKOERU_CONFIG: /config/config.toml
      KIKOERU_LISTEN: ""
    ports: ["127.0.0.1:8888:9000"]
    healthcheck:
      test: ["CMD", "/bin/env", "KIKOERU_LISTEN=127.0.0.1:9000", "/bin/kikoeru-neo-healthcheck"]
```

The image has no `/bin/sh`, so use an argument list rather than a shell-form health check. A healthy container does not prove that an administrator exists or that remote storage is reachable.
