# 运行单文件版

每个发布都附带一个适用于 `x86_64` Linux 的 `kikoeru-neo` 可执行文件。它包含服务端、网页界面和 SQLite，不依赖任何共享库，在包括 Alpine 在内的各发行版上都能运行。它不包含 ffmpeg 和 ffprobe，开始之前要先从发行版安装这两个工具。`aarch64` 等其他平台请按[参与开发](../../CONTRIBUTING.zh-CN.md#不用-nix-构建完整服务端)从源码构建。

## 先安装 ffmpeg

服务端用 `ffmpeg` 和 `ffprobe` 探测时长、转码、播放 HLS 视频，以及分析响度和波形。从发行版安装这两个工具：

| 发行版 | 命令 |
|---|---|
| Debian、Ubuntu | `sudo apt install ffmpeg` |
| Fedora | 先启用 [RPM Fusion](https://rpmfusion.org/Configuration)，再运行 `sudo dnf swap ffmpeg-free ffmpeg --allowerasing` |
| Arch Linux | `sudo pacman -S ffmpeg` |
| Alpine | `sudo apk add ffmpeg` |
| openSUSE | 从 Packman 软件源安装 `ffmpeg` |

Fedora 默认的 `ffmpeg-free` 和 openSUSE 默认的软件包都不带 H.264 编码器 `libx264`，视频转码需要它。安装后检查编码器：

```sh
ffmpeg -hide_banner -encoders | grep -E ' (libx264|aac) '
ffprobe -version | head -n 1
```

输出里必须同时有 `libx264` 和 `aac`。工具不在 `PATH` 里时，在[配置](configuration.md#参数参考)中把 `media.ffmpeg` 和 `media.ffprobe` 设为它们的绝对路径。

没有 ffmpeg 时服务端仍能启动，浏览器能直接播放的格式也能播放，但时长探测、转码、HLS 视频播放以及响度和波形分析都会失败。`serve` 启动时会在后台检查这两个工具，每发现一个问题就记一条警告，例如：

```text
media tool check: ffmpeg "ffmpeg": missing encoder libx264; affected features: HLS video transcoding; install a complete ffmpeg build or set media.ffmpeg in config.toml
```

出现警告后服务照常运行。装好完整的 ffmpeg 或配置好路径后重启服务即可。两个工具都完整时不会有警告。

## 下载并校验

```sh
version=0.1.0
asset="kikoeru-neo-$version-x86_64-linux-musl"
base="https://github.com/QWERChin/kikoeru-neo/releases/download/v$version"
curl -fLO "$base/$asset"
curl -fLO "$base/SHA256SUMS"
sha256sum --check SHA256SUMS
sudo install -m 0755 "$asset" /usr/local/bin/kikoeru-neo
kikoeru-neo --version
```

`sha256sum --check` 必须输出 `OK`。校验和不一致的文件不要运行。

## 以自己的用户试用

准备独立的数据目录和配置。以下命令以普通用户运行，不要用 root：

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

创建账号时输入两次密码，至少 8 个字符。打开 `http://127.0.0.1:8888` 登录，在“设置”中添加本地媒体库，路径填当前用户能读取的目录，例如 `/srv/media/works`。首次扫描和播放见[快速开始](quickstart.md#扫描并播放)。按 Ctrl-C 停止服务。不要删除 `demo-data`，它保存账号、封面和进度。

## 作为系统服务运行

仓库带有一个 systemd 单元 `packaging/systemd/kikoeru-neo.service`，它和下面的命令一起在虚拟机里测试过。它以系统用户 `kikoeru-neo` 运行服务，读取 `/etc/kikoeru-neo/config.toml`，数据放在 `/var/lib/kikoeru-neo`（由 systemd 以 `0700` 权限创建），并通过默认的 `PATH` 找到 `/usr/bin` 里的 ffmpeg。服务只能写自己的数据目录。本地媒体放在哪里都可以读，包括 `/home` 下，只要 `kikoeru-neo` 用户有读取权限。

创建用户和配置，再安装并启动单元：

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
curl -fLO https://raw.githubusercontent.com/QWERChin/kikoeru-neo/v0.1.0/packaging/systemd/kikoeru-neo.service
sudo install -m 0644 kikoeru-neo.service /etc/systemd/system/kikoeru-neo.service
sudo systemctl daemon-reload
sudo systemctl enable --now kikoeru-neo.service
curl --fail --retry 30 --retry-delay 1 --retry-connrefused \
  http://127.0.0.1:8888/api/ready
```

等 `/api/ready` 返回后再做下一步，首次启动要先创建数据库。`user`、`backup` 等管理命令必须以服务用户身份、用同一份配置和私有的 umask 运行，否则它们创建的文件属主或权限会不对。在管理员的 shell 里定义下面的函数，或者把它加进这个用户的 `~/.bashrc`：

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

`env -i` 让自己 shell 里的 `KIKOERU_DATABASE_URL` 等变量不会带进命令。输入两次密码。然后在服务器上打开 `http://127.0.0.1:8888`，或者通过[反向代理](security.md)从其他设备访问，再按[媒体库与扫描](libraries.md)添加媒体库。

查看日志，包括 ffmpeg 检查的结果：

```sh
sudo journalctl -u kikoeru-neo.service -f
sudo journalctl -u kikoeru-neo.service -p warning -b
```

[账号](accounts.md)和[备份与恢复](backup-restore.md)用的也是这个 `kikoeru_neo` 函数。

## 升级

替换二进制之前先读[升级](upgrade.md)。新版本启动时可能迁移数据库，而旧版本会拒绝更新过的数据库结构，所以在新版本正常运行之前，保留[备份](backup-restore.md)和旧的二进制。
