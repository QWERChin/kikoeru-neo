# 快速开始

用发布的 Docker 镜像启动服务，创建管理员，几分钟内扫描一个媒体库并开始播放。需要一台 `x86_64` Linux 机器，装有 Docker Engine 25 或更新版本，再准备一份自己有权读取的媒体目录。镜像已经包含 ffmpeg 和 ffprobe，不需要另装。试用之后要长期运行时，转到 [Docker Compose 安装](install-docker.md)。

作品文件夹的名字必须包含 `RJ`、`BJ` 或 `VJ` 加 6 到 8 位数字，例如 `RJ01234567 示例作品`。这只是合成编号示例，不表示线上存在对应作品，详细规则见[媒体库](libraries.md#准备作品目录)。

## 创建账号并启动

选一个工作目录，准备一份空配置。媒体库之后在网页上添加：

```sh
mkdir -p neo-trial
cd neo-trial
printf '# 媒体库可在登录后的设置页添加。\n' > config.toml
chmod 0644 config.toml
```

先在新卷里创建管理员，再启动服务，并把媒体目录只读挂进容器。把 `/srv/media/works` 换成自己的目录：

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

创建账号时输入两次密码，至少 8 个字符，成功后会输出 `created admin admin`。容器以 uid 1000 运行，这个用户必须能读取媒体目录。运行 `docker ps`，等容器显示 `healthy`。

`aarch64` 机器没有发布的镜像，按 [Docker Compose 安装](install-docker.md#用-dockerfile-构建)自己构建，再把 `image` 设成构建时用的标签。

## 扫描并播放

1. 在这台机器的浏览器打开 `http://127.0.0.1:8888`，用刚建的账号登录。
2. 进入“设置”，点“添加媒体库”，选择本地类型，填写容器里的路径 `/media/works`，不是宿主机路径。
3. 保存后点该媒体库的“扫描”。任务一次运行一个，离开设置页不会取消扫描。
4. 回到“资料库”，打开作品，选择轨道播放。先确认能发声、拖动进度，再刷新页面核对进度。

本地音视频的时长探测单独排队，扫描结束后可能还没完成。抓不到元数据时，作品仍以目录名入库并可播放。没有作品出现时，先查[目录命名和扫描限制](libraries.md)，不要急着改别的设置。

## 停止、继续使用或删除

```sh
docker stop neo-trial
docker start neo-trial
docker logs --tail 100 neo-trial
```

卷 `neo-trial-data` 保存账号、封面和进度，`docker stop` 不会删除它。要彻底删除试用实例，运行 `docker rm -fv neo-trial` 和 `docker volume rm neo-trial-data`。

需要在另一台设备访问时，按[安全与网络](security.md)配置监听、防火墙及 HTTPS。要长期使用，改用 [Docker Compose](install-docker.md)，它另外提供备份卷、只读根文件系统和自动重启。

## 不用 Docker

- [单文件版](install-binary.md)是一个适用于 `x86_64` Linux 的可执行文件，需要先从发行版安装 ffmpeg 和 ffprobe。
- 有 Nix 时，`nix build github:QWERChin/kikoeru-neo/v0.1.0#neo-server` 生成 `result/bin/kikoeru-neo`。它自带 ffmpeg，用法与单文件版相同。[NixOS 安装](install-nixos.md)说明如何配置系统服务。
