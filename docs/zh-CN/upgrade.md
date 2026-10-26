# 升级与恢复旧版本

升级前固定目标版本，检查是否有数据库迁移，并保存完整备份和对应的旧软件。服务启动会自动迁移数据库，旧程序可能拒绝打开新 schema，不能把“换回旧二进制”当作完整回滚。

## 准备升级

1. 保存当前版本和部署配置。可在浏览器打开服务的 `/api/version`，记录版本及可用的构建信息。
2. 选择目标版本，阅读变更说明，并检查目标源码的数据库迁移是否变化。没有充分说明时，按会迁移处理，先[备份](backup-restore.md)。
3. 在设置页的任务列表确认没有运行或排队的扫描。停止触发新任务的操作，必要时暂时关闭自动扫描和目录监听。

运行中的扫描会被重启中断，启动后标为 `interrupted`，需要手动重新扫描。排队任务会重新排队，不因重启自动取消。已经提交的数据保留，但这不是自动断点续跑保证。

## Docker

用完整版本标签（如 `0.1.1`）或镜像摘要作为恢复点，不用浮动的 `0.1` 系列标签。改动前先记下容器当前使用的镜像：

```sh
docker compose images kikoeru-neo
```

使用发布镜像时，在 `compose.yaml` 里改好标签，完成备份后拉取并重启：

```sh
docker compose pull kikoeru-neo
docker compose up -d
docker compose ps
docker compose logs --tail 100 kikoeru-neo
```

自己构建镜像时，检出新版本的标签，用和之前相同的方式（[Dockerfile 或 Nix](install-docker.md#获取镜像)）构建到新标签，例如 `kikoeru-neo:0.1.1`，再改 `compose.yaml` 并运行同样的 `up -d`。保留旧镜像，不要用新构建覆盖旧标签，这样才能回到旧版本。升级不需要删除数据卷。

## 单文件版

按[下载并校验](install-binary.md#下载并校验)获取新版本，但保留旧文件。停止服务，用新文件替换 `/usr/local/bin/kikoeru-neo`，再用同一份配置和数据目录启动服务。新版本运行正常之前，不要删除旧的二进制。

## NixOS

NixOS 用 `flake.lock` 固定 kikoeru-neo 输入。把输入指向目标版本的标签，例如 `github:QWERChin/kikoeru-neo/v0.1.1`，保存更新后的锁文件，再从同一份配置激活系统。不要让另一份旧锁文件在下次部署时覆盖它。

在系统 flake 所在目录，目标名为 `media` 时：

```sh
nix flake update kikoeru-neo
sudo nixos-rebuild switch --flake .#media
systemctl status kikoeru-neo
journalctl -u kikoeru-neo -n 100 --no-pager
```

`nix flake update` 选择输入地址允许的最新修订，不会替你挑选合适的发布。需要精确版本时先修改输入地址。不要在确认目标和备份前直接运行升级命令。

## 升级后核对

`/api/ready` 应返回 HTTP 200，`/api/version` 应显示目标版本。然后登录，核对媒体库连通、封面、播放和进度。就绪接口不会逐个验证媒体库，仍需实际播放。中断扫描在确认存储正常后重新发起。

桌面应用没有自动更新，按[桌面应用](desktop.md)获取新包，先完全退出旧应用，再更换安装包。服务器模式加载服务器提供的页面，升级服务器和桌面外壳时都要检查两者兼容。

## 恢复旧版本

先停止新服务并保留故障数据，恢复备份到空目录或空数据库，然后用与该备份匹配的软件和配置启动。旧程序拒绝新 schema 时，不要改迁移记录或删表绕过检查。具体恢复步骤见[备份与恢复](backup-restore.md)。

恢复会丢失备份之后的新状态。保留故障副本便于之后核对，但不要同时启动两个实例写同一个数据目录或数据库。
