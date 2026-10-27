# 用 Docker Compose 运行

把 kikoeru-neo 作为长期运行的容器部署，配好持久卷、备份卷和管理员。需要 Linux 上的 Docker Engine 25 或更新版本，以及 Docker Compose v2。镜像包含服务端、网页界面、ffmpeg 和 ffprobe，宿主机不需要另装其他东西。macOS 和 Windows 上的 Docker Desktop 尚未测试。

## 获取镜像

下面三种方式任选其一。它们得到的镜像结构相同，本页其余步骤对三种方式都一样。

### 拉取发布的镜像

发布的镜像只有 `x86_64` Linux 版本：

```sh
docker pull ghcr.io/qwerchin/kikoeru-neo:0.1.1
```

`0.1.1` 固定一个版本，`0.1` 跟随 0.1 系列最新的补丁版本。仓库不发布 `latest`，只有改了标签才会升级。`aarch64` 没有发布的镜像，改用 Dockerfile 构建。

### 用 Dockerfile 构建

只需要 Git 和 Docker，`x86_64` 和 `aarch64` Linux 都可以。Dockerfile 在构建容器里运行 Nix，得到的就是 CI 测试的那份构建：

```sh
git clone --branch v0.1.1 https://github.com/QWERChin/kikoeru-neo.git
cd kikoeru-neo
docker build -t kikoeru-neo:0.1.1 .
```

第一次构建要从源码编译 Rust 服务端和精简版 ffmpeg，在一台 24 核的测试机上用了大约 10 分钟。构建缓存约占 10 GB 磁盘，构建完可以用 `docker builder prune` 删除，最终镜像约 175 MB。源码没有任何改动时，再次构建才能复用 Docker 的层缓存。Dockerfile 构建不带 Git 信息，`/api/version` 只报告版本号，没有提交号。下面的 Compose 文件里把 `image` 改成 `kikoeru-neo:0.1.1`。

### 用 Nix 构建

Linux 上装有支持 flakes 的 Nix 时，可以不经 Docker 构建镜像，再载入：

```sh
nix build github:QWERChin/kikoeru-neo/v0.1.1#docker-image --out-link docker-image
./docker-image | docker load
```

`docker-image` 是输出镜像流的脚本，不是 tar 文件。载入后镜像名为 `kikoeru-neo:0.1.1`，下面的 Compose 文件里把 `image` 改成这个名字。

## 准备 Compose

新建一个部署目录，在其中保存 `compose.yaml` 和 `config.toml`。先用空配置启动，再从网页添加媒体库：

```sh
mkdir -p neo-deploy
cd neo-deploy
printf '# 媒体库可在登录后的设置页添加。\n' > config.toml
chmod 0644 config.toml
```

将下面的内容保存为 `compose.yaml`：

```yaml
services:
  kikoeru-neo:
    image: ghcr.io/qwerchin/kikoeru-neo:0.1.1
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

`172.30.250.0/24` 是可用的私有子网示例，运行前确认它不与自己的网络冲突。不要因为示例指定了网关就把网关加入 `trusted_proxies`，实际代理来源要另行核对。

`KIKOERU_CONFIG` 让 `user`、`backup` 等命令和 `serve` 使用同一份配置。镜像本身只给 `serve` 指定 `/config/config.toml`，不会替其他子命令选择这个文件。

使用本地媒体库时，在服务的 `volumes` 下另外加入以下只读挂载，并先确认宿主机目录存在且 uid 1000 可读：

```yaml
      - type: bind
        source: /srv/media/works
        target: /media/works
        read_only: true
        bind: { create_host_path: false }
```

界面中的本地路径填 `/media/works`，不是宿主机路径。只用 OpenList 或 WebDAV 时不需要媒体挂载。

## 创建管理员并启动

已有旧容器需要迁移时，跳过本节，转到[迁移旧数据](migration.md#从旧容器迁移)，迁移脚本会自己创建空的目标卷。

全新安装时运行：

```sh
docker volume create kikoeru-neo-data
docker compose run --rm kikoeru-neo user create --admin admin
docker compose up -d
docker compose ps
docker compose logs --tail 100 kikoeru-neo
```

交互输入密码后应看到 `created admin admin`。等待服务显示 `healthy`，在 Docker 宿主机打开 `http://127.0.0.1:8888`。远程服务器可以使用 [NixOS 安装页中的 SSH 转发方式](install-nixos.md#激活创建管理员打开页面)，或按[安全与网络](security.md)接入反向代理。登录后按[媒体库与扫描](libraries.md)完成首次播放。

## 日常管理

在部署目录运行 `docker compose logs -f kikoeru-neo` 查看日志。账号、备份和升级分别见[账号](accounts.md)、[备份与恢复](backup-restore.md)和[升级](upgrade.md)。不要删除数据卷，`docker compose down -v` 可能删除非外部卷，包括示例中的备份卷。

容器以 `1000:1000` 运行。新命名卷会继承镜像中 `/data` 和 `/backup` 的 `0700` 权限。替换为 bind mount 时，必须安排容器内 uid 1000 的读写权限。rootless Docker 的宿主机 UID 映射与普通 Docker 不同，按容器内身份核对，不能一律在宿主机 `chown 1000`。数据目录和 SQLite 文件不能同组可写，NAS 的 `0777` 数据卷不适用，媒体目录则只需可读。

PostgreSQL 可在 `config.toml` 中配置 `database.url`，因为这里已经为所有子命令设置 `KIKOERU_CONFIG`。也可以通过 `KIKOERU_DATABASE_URL` 覆盖。密码不写进示例文件或镜像，见[配置](configuration.md)。

## 监听与健康检查

镜像默认设置 `KIKOERU_LISTEN=0.0.0.0:8888`，它优先于配置文件的 `listen`。一般只改 `ports` 的宿主机端口。确实需要改容器端口时，同步修改 `KIKOERU_LISTEN` 和端口映射，镜像的健康检查会跟着环境变量探测。

健康检查每 30 秒请求 `/api/ready`，超时 10 秒，连续失败 3 次判为不健康，初次启动有 120 秒宽限期。若清空 `KIKOERU_LISTEN` 改用文件中的监听地址，必须为健康检查单独给出地址，例如服务在容器内监听 `0.0.0.0:9000` 时：

```yaml
    environment:
      KIKOERU_CONFIG: /config/config.toml
      KIKOERU_LISTEN: ""
    ports: ["127.0.0.1:8888:9000"]
    healthcheck:
      test: ["CMD", "/bin/env", "KIKOERU_LISTEN=127.0.0.1:9000", "/bin/kikoeru-neo-healthcheck"]
```

镜像没有 `/bin/sh`，覆盖健康检查时用上述参数列表，不用依赖 shell 的字符串命令。健康检查成功不代表管理员已创建或远端媒体库可用。
