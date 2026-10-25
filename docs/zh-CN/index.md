# kikoeru-neo 使用指南

kikoeru-neo 用来浏览和播放自己保存的有声剧、有声书及视频，并从 DLsite 获取作品信息。这套指南面向自行安装和运行服务的管理员，也介绍桌面应用。界面目前使用中文。

需要准备可读取的媒体目录，或者 OpenList、WebDAV 服务。扫描器按作品目录名中的编号识别作品，普通的无编号文件夹不会自动成为作品。请先看[快速开始](quickstart.md)，再按运行方式选择安装页。

## 安装与首次使用

| 你的任务 | 从这里开始 |
|---|---|
| 在 Linux 上先试用 | [快速开始](quickstart.md) |
| 作为 NixOS 系统服务运行 | [NixOS 安装](install-nixos.md) |
| 用 Docker Compose 运行 | [Docker 安装](install-docker.md) |
| 在个人电脑上使用独立窗口或本机媒体库 | [桌面应用](desktop.md) |
| 从 kikoeru-express 保留收藏、评价和进度 | [迁移旧数据](migration.md) |

## 日常管理

- [配置](configuration.md)列出配置文件、环境变量、默认值和数据库选择。
- [媒体库与扫描](libraries.md)说明目录命名、存储账号、手动扫描和自动扫描。
- [账号](accounts.md)说明创建用户、重设密码和撤销登录。
- [备份与恢复](backup-restore.md)说明完整备份范围和隔离恢复验证。
- [升级](upgrade.md)说明版本固定、数据库迁移和恢复旧版本。
- [故障排查](troubleshooting.md)按登录、扫描和播放症状组织检查步骤。
- [安全与网络](security.md)说明 HTTPS、反向代理、明文 HTTP 和凭据边界。

这里把包含作品文件夹的根目录称为“媒体库”，单部作品所在的文件夹称为“作品目录”，聚合浏览所有作品的页面称为“资料库”。示例中的账号、编号、目录和地址都是合成值，部署时换成自己的值。

设计书 `book/` 仅有中文，面向实现者，不是安装前提。项目使用 GPL-3.0-or-later 许可证。
