# kikoeru-neo

自托管的有声剧和有声书播放器，从 DLsite 获取作品信息。可以通过手机、桌面浏览器或桌面应用浏览和播放。项目是独立重写的实现，可以读取旧 kikoeru-express 的数据格式。

界面目前使用简体中文，截图中的媒体均为合成数据。

![桌面浏览器中的资料库](docs/screenshots/desktop-library.webp)

<table>
  <tr>
    <td><img src="docs/screenshots/phone-library.webp" alt="手机上的资料库"></td>
    <td><img src="docs/screenshots/phone-detail-playing.webp" alt="播放中的作品详情"></td>
    <td><img src="docs/screenshots/phone-player-dark.webp" alt="深色模式的全屏播放器"></td>
  </tr>
</table>

<details>
<summary>更多截图</summary>

![带字幕的全屏播放页](docs/screenshots/desktop-player.webp)
![带 ASS 字幕的视频](docs/screenshots/desktop-video.webp)
![扫描排队中的设置页](docs/screenshots/desktop-settings.webp)

</details>

## 功能

- 按封面浏览作品，按社团、声优和标签搜索，并保存收藏和评价。
- 播放进度保存在服务器上，可以换设备继续收听。后台播放和媒体控制取决于浏览器与操作系统。
- 读取本地目录、OpenList 和 WebDAV 媒体库，支持定时扫描和可选的本地文件监听。
- 从 DLsite 获取元数据和封面，普通刷新保留手动修改的内容。
- 音频在需要时转码，视频支持 HLS 回退和 ASS 字幕。音频还支持波形和响度均衡。
- 通过浏览器使用，在受支持的安全上下文中安装 PWA，或通过桌面应用连接。

## 快速开始

在装有 Docker Engine 25 或更新版本的 `x86_64` Linux 上，创建管理员并启动服务。把 `/srv/media/works` 换成自己的媒体目录：

```sh
mkdir -p neo-trial && cd neo-trial
printf '# 媒体库可在登录后的设置页添加。\n' > config.toml
chmod 0644 config.toml
image=ghcr.io/qwerchin/kikoeru-neo:0.1.0
docker run --rm -it -e KIKOERU_CONFIG=/config/config.toml \
  -v neo-trial-data:/data -v "$PWD/config.toml:/config/config.toml:ro" \
  "$image" user create --admin admin
docker run -d --name neo-trial -p 127.0.0.1:8888:8888 \
  -v neo-trial-data:/data -v "$PWD/config.toml:/config/config.toml:ro" \
  -v /srv/media/works:/media/works:ro \
  "$image"
```

按提示输入至少 8 个字符的密码。打开 `http://127.0.0.1:8888`，登录后在设置页添加本地媒体库，路径填 `/media/works`。作品目录名必须包含 `RJ`、`BJ` 或 `VJ` 加 6 到 8 位数字，例如合成名称 `RJ01234567 示例作品`。保存媒体库并启动扫描，然后打开作品播放轨道。

镜像已包含 ffmpeg 和 ffprobe。停止和删除试用实例的方法见[快速开始](docs/zh-CN/quickstart.md)，向其他设备开放服务前先读[安全与网络](docs/zh-CN/security.md)。核心播放支持 HTTP，Service Worker 和 PWA 安装需要受支持的安全上下文。

## 安装方式

- [Docker Compose](docs/zh-CN/install-docker.md)，可以用发布的镜像，也可以用 Dockerfile 或 Nix 自己构建。
- [单文件版](docs/zh-CN/install-binary.md)，适用于任意 `x86_64` Linux 发行版，需要先从发行版安装 ffmpeg 和 ffprobe。
- [NixOS](docs/zh-CN/install-nixos.md) 系统服务。
- [桌面应用](docs/zh-CN/desktop.md)，用于独立窗口或个人电脑上的本机媒体库。

## 使用指南

- [使用指南首页](docs/zh-CN/index.md)。
- [配置](docs/zh-CN/configuration.md)、[媒体库与扫描](docs/zh-CN/libraries.md)和[账号](docs/zh-CN/accounts.md)。
- [从 kikoeru-express 迁移](docs/zh-CN/migration.md)、[备份与恢复](docs/zh-CN/backup-restore.md)和[升级](docs/zh-CN/upgrade.md)。
- [故障排查](docs/zh-CN/troubleshooting.md)。

## 参与贡献

开发、测试和拉取请求的要求见 [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md)。仓库带有装好全部工具的 Dev Container，只有 Docker 也能开始开发。设计书只维护中文版，面向实现者，安装和使用播放器不需要先读设计书。

## 许可证

GPL-3.0-or-later，见 [LICENSE](LICENSE)。
