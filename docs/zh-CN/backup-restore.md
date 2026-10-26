# 备份与恢复

用 `kikoeru-neo backup` 备份数据库和应用状态，恢复时使用空目录及匹配的软件版本。备份包含远端凭据和用户状态，应只允许备份管理者读取。原始媒体、外部配置和外部密码文件需要另行备份。

## 备份范围

| 内容 | 应用备份是否包含 |
|---|---|
| 数据库 | 包含。SQLite 或 PostgreSQL 都导出成 `neo.sqlite3`。 |
| `secrets/` | 包含签名密钥和界面保存的 OpenList、WebDAV 密码。 |
| `covers/`、`lyrics/` | 包含封面和用户保存的字幕。 |
| `import-report.json` | 存在时包含。 |
| `cache/` | 不包含，可按需重建。 |
| `config.toml`、NixOS 配置、Compose、外部 `password_file` | 不包含，另行保存，并保护其中的凭据。 |
| 本地或远端原始媒体 | 不包含，由媒体存储自己的备份策略负责。 |

程序跳过符号链接，不把链接外面的文件带入备份。检查输出中的跳过数量，不能把被跳过的状态误认为已备份。

## 创建备份

备份可在服务运行时执行。目标必须还不存在，父目录必须存在且运行账号可写，目标不能位于数据目录里面。每次用一个新名字，下面的 `before-upgrade` 只适合首次使用。

Docker 安装页的 Compose 有独立备份卷，在部署目录运行：

```sh
docker compose exec -T kikoeru-neo kikoeru-neo backup /backup/before-upgrade
umask 077
mkdir backup-export
docker compose cp kikoeru-neo:/backup/before-upgrade ./backup-export/
```

`backup-export` 是本次新建的目录，用于限制宿主机上的访问。复制后检查文件存在、属主正确，再把备份保存到独立存储。容器健康不代表备份成功，以备份命令退出状态和文件为准。

[单文件版](install-binary.md#作为系统服务运行)作为系统服务运行时，使用那里定义的 `kikoeru_neo` 函数：

```sh
sudo install -d -o kikoeru-neo -g kikoeru-neo -m 0700 /var/backups/kikoeru-neo
kikoeru_neo backup /var/backups/kikoeru-neo/before-upgrade
```

NixOS 默认安装可先准备备份父目录：

```sh
sudo install -d -o kikoeru-neo -g kikoeru-neo -m 0700 /var/backup/kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo backup /var/backup/kikoeru-neo/before-upgrade
```

SQLite 通过 `VACUUM INTO` 导出包含 WAL 中数据的一致数据库快照，不能替换成运行时只复制 `neo.sqlite3`。PostgreSQL 在只读快照事务中导出为 SQLite，要求源数据库迁移版本与程序一致，不需要 `pg_dump`。SQLite 备份命令不运行迁移。

## 恢复到 SQLite

1. 停止服务，或通过桌面托盘完全退出应用。保留当前故障状态，恢复到另一个空目录，不把备份覆盖进仍有 WAL 文件的旧目录。
2. 复制备份中的 `neo.sqlite3`、`secrets/`、`covers/`、`lyrics/` 及存在时的 `import-report.json`。不要只恢复数据库，否则签名密钥、远端密码和封面可能不对应。
3. 把目录属主设为服务运行账号，目录权限设为 `0700`，文件设为 `0600`。Docker 按容器内 uid 1000 安排属主，rootless 环境需考虑 UID 映射。
4. 恢复对应的外部配置和密码文件。服务必须指向恢复目录，并确保没有仍指向其他 PostgreSQL 的配置或环境变量。
5. 先用备份时的软件版本启动，再核对登录、封面、收藏和进度。需要升级时按[升级](upgrade.md)另行操作。

恢复后数据回到备份时刻，之后新增的进度和收藏不会出现。签名密钥和用户 token 版本也回到备份时刻，恢复完成后按需要重新设密码或退出所有设备。

## 恢复到 PostgreSQL

使用独立的空目标数据库，停止将要使用它的应用实例。用与备份 schema 匹配的服务器包复制，例如目标角色是 `kikoeru-neo`、数据库是 `neo_restored`：

```sh
sudo -u kikoeru-neo kikoeru-neo db copy \
  --from /var/backup/kikoeru-neo/before-upgrade/neo.sqlite3 \
  --to 'postgres:///neo_restored?host=/run/postgresql&user=kikoeru-neo'
```

目标数据库和角色需预先创建并安排权限。不要复用正在服务的数据库。命令成功后，恢复其余状态目录，配置 `database.url` 为这个目标，再启动服务。源备份保持不变，具体复制限制见[更换数据库](configuration.md#更换数据库)。

## 在隔离环境验证恢复

使用独立虚拟机或容器，不挂载原始媒体，不允许访问正式数据库、远端存储和元数据服务。复制品不是只读的，数据库中可能保存媒体库、凭据、自动扫描时间和排队任务，启动后可能立即扫描。只关配置文件里的自动扫描不能清除数据库里的设置。

在这个隔离环境中把备份复制到 `/srv/neo-restore`，并保存以下独立文件为 `/srv/neo-restore-config.toml`：

```toml
listen = "127.0.0.1:8899"

[metadata]
source = "none"
```

以副本拥有者运行：

```sh
env -u KIKOERU_DATABASE_URL KIKOERU_LISTEN=127.0.0.1:8899 \
  kikoeru-neo --data-dir /srv/neo-restore serve --config /srv/neo-restore-config.toml
```

显式 `serve --config` 覆盖 NixOS 包装器的默认配置，清除数据库环境变量避免连回 PostgreSQL。运行 `user` 或 `backup` 时没有这个 `--config`，必须改用显式 `KIKOERU_CONFIG=/srv/neo-restore-config.toml`，并同样清除 `KIKOERU_DATABASE_URL`。

检查就绪接口、账号、收藏、进度和封面。网络隔离下远端库不可用是预期现象，需要验证播放时仅添加合成媒体副本。验证完成后停止实例，再删除演练副本，保留原备份及验证记录。
