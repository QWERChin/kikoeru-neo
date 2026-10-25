# 配置服务

服务读取 TOML 配置文件，账号、媒体库状态和用户进度保存在数据库中。先确定服务实际使用的数据目录和配置文件，再修改配置。文件配置只在启动时读取，修改后需要重启，重启前检查[扫描任务](upgrade.md)。

## 选择配置和数据目录

| 设置 | 优先顺序 |
|---|---|
| 数据目录 | `--data-dir`、非空 `KIKOERU_DATA_DIR`。都没有时退出，不使用当前目录。 |
| 配置文件 | `serve --config`、非空 `KIKOERU_CONFIG`、数据目录中的 `config.toml`。 |
| PostgreSQL 地址 | 非空 `KIKOERU_DATABASE_URL`、`database.url`。都没有时用数据目录中的 `neo.sqlite3`。 |
| 监听地址 | 非空 `KIKOERU_LISTEN`、`listen`、`127.0.0.1:8888`。只有 `serve` 读取该环境变量。 |
| 日志级别 | `RUST_LOG`、`log.level`、`info`。 |

默认位置没有 `config.toml` 时使用默认值。明确指定的配置文件不存在、未知配置项或非法值都会使启动失败。非空环境变量覆盖文件中的同类设置，但文件仍需通过解析和校验。

`user`、`backup`、`migrate` 和实际执行的 `import-legacy` 也读取 `KIKOERU_CONFIG`，但没有新服务配置的 `--config` 参数。`db copy` 只在选择默认源数据库时读取它，显式的 `--from` 和 `--to` 直接决定复制的两端。`import-legacy --config` 指的是旧安装的 `config.json`，不要混用。只读预检 `import-legacy --dry-run` 不需要目标数据目录。

NixOS 包装器已默认选择服务配置。Docker 应采用[安装页](install-docker.md)中的 `KIKOERU_CONFIG` 设置。单改 `--data-dir` 不会覆盖 PostgreSQL 地址，恢复副本必须[隔离配置](backup-restore.md#在隔离环境验证恢复)。

## 写一份最小配置

```toml
listen = "127.0.0.1:8888"

[cache]
max_bytes = "20GB"
min_free_bytes = "5GB"
```

用数据目录的拥有者启动，例如已经把文件保存为 `/etc/kikoeru-neo/config.toml`，并且该账号可写 `/srv/kikoeru-neo` 时：

```sh
KIKOERU_CONFIG=/etc/kikoeru-neo/config.toml \
  kikoeru-neo --data-dir /srv/kikoeru-neo serve
```

日志会显示读取的配置文件和监听地址。浏览器打开监听地址应出现登录页。媒体库可以稍后在设置页添加，见[媒体库](libraries.md)。`public_url` 当前没有功能读取它，不配置也能正常运行。设置它不会建立 DNS、申请证书、打开端口或配置反向代理。

## 参数参考

下表是服务器默认值。桌面应用的监听、数据库和日志有独立约定，见[桌面应用](desktop.md)。

| 参数 | 默认值 | 用途 |
|---|---|---|
| `listen` | `"127.0.0.1:8888"` | 监听 IP 和端口，不接受主机名。 |
| `public_url` | 未设置 | 可解析的 HTTP 或 HTTPS 地址，目前未用于生成链接。 |
| `database.url` | 未设置 | 设置后使用 PostgreSQL，接受 `postgres://` 和 `postgresql://`。 |
| `trusted_proxies` | `[]` | 信任哪些代理提供登录限流的客户端地址，只接受单个 IP。 |
| `roots` | `[]` | 声明式媒体库，字段见[媒体库](libraries.md)。 |
| `media.heavy_jobs` | `1` | 同时运行的转码任务数。 |
| `media.light_jobs` | `4` | 后台探测、响度等轻任务的并发数。播放时的即时探测另有同样多的名额。 |
| `media.job_timeout` | `"2h"` | 单个转码任务的超时。 |
| `media.queue_limit` | `32` | 排队任务的上限，转码和响度、波形分析分别计数，某一类超过时返回 `queue_full`。 |
| `media.ffmpeg`、`media.ffprobe` | 未设置 | 可执行文件的绝对路径。未设置时从 `PATH` 查找，Nix 包和镜像已提供。 |
| `media.video_transcode` | `true` | 是否允许视频重新编码，不影响直接播放和转封装。 |
| `cache.max_bytes` | `"20GB"` | `cache/transcode/` 的淘汰目标，按最近访问时间淘汰。正在使用的条目可能让占用超过它，临时文件和缩略图不计入。 |
| `cache.min_free_bytes` | `"5GB"` | 剩余磁盘空间不足时拒绝新的生成任务。 |
| `log.level` | `"info"` | 可选 `trace`、`debug`、`info`、`warn`、`error`。 |
| `metadata.source` | `"dlsite"` | `"none"` 关闭元数据联网抓取，新作品使用目录名。 |
| `metadata.fallback` | `"none"` | `"asmr_one"` 为 RJ 作品启用备用来源，DLsite 抓取失败时尝试。 |
| `metadata.hvdb` | `false` | 为缺少声优信息的 RJ 作品启用 hvdb 补充。 |
| `metadata.dlsite_url` | `"https://www.dlsite.com/"` | DLsite 服务地址，通常不改。 |
| `metadata.asmr_one_urls` | 见下文 | 备用来源的镜像顺序，列表不能为空。 |
| `metadata.hvdb_url` | `"https://hvdb.me/"` | hvdb 服务地址，通常不改。 |

asmr.one 默认按 `https://api.asmr.one/`、`https://api.asmr-200.com/`、`https://api.asmr-100.com/`、`https://api.asmr-300.com/` 的顺序探测。`metadata.source = "none"` 时，备用来源和 hvdb 都不起作用。这些是代码配置值，不保证第三方服务一直可用。

大小接受整数字节数或 `"512MiB"` 这类字符串，不能为负。`KB`、`MB`、`GB` 按 1000 进位，`KiB`、`MiB`、`GiB` 按 1024 进位。时长使用 `"90m"`、`"2h"` 等字符串。任务并发和队列上限必须是正整数，任务超时不能为零。

## 网页设置和文件配置

在网页添加的媒体库保存在数据库中，修改立即生效，不改写 TOML。远端密码写入 `secrets/roots/<id>.password`。文件配置的媒体库在网页中只读。

管理员设置页的“网络代理”也保存在数据库中，修改立即影响元数据请求，不是 `config.toml` 的参数。它用于 DLsite、asmr.one 和 hvdb，不代理 OpenList 或 WebDAV 媒体文件。代理地址从运行后端的机器访问，桌面本机模式才是个人电脑。凭据和传输限制见[安全与网络](security.md)。

## NixOS 模块选项

这些选项位于 `services.kikoeru-neo`：

| 选项 | 默认值 | 说明 |
|---|---|---|
| `enable` | `false` | 启用系统服务。 |
| `package` | flake 的默认服务端包 | 包装器提供 ffmpeg 和 ffprobe。 |
| `dataDir` | `/var/lib/kikoeru-neo` | 模块创建 `0700` 数据目录。 |
| `user`、`group` | `kikoeru-neo` | 默认用户和组由模块创建，改成其他名字时自行声明对应账号或组。 |
| `database.createLocally` | `false` | 创建本机 PostgreSQL 数据库和角色，使用 peer 认证。 |
| `settings` | `{}` | 生成 TOML，不能写密码，因为内容进入 Nix store。 |

## 更换数据库

服务器默认使用 SQLite。改用 PostgreSQL 前，准备归服务账号所有的专用数据库。密码通过 `PGPASSWORD` 或权限受限的 `.pgpass` 提供，也可用 `PGPASSFILE` 指定文件，不把密码放进命令行。远程数据库需要的 TLS 参数可以写在 URL 中，`sslmode=verify-full` 校验证书，私有 CA 用 `sslrootcert=` 指定。`sslmode=require` 只加密，不验证证书身份。

1. 用当前版本完成[备份](backup-restore.md)，检查没有运行或排队的扫描，再停止服务。
2. 用支持 PostgreSQL 的服务器包执行 `db copy`。源 schema 必须和程序版本一致，目标 SQLite 文件必须不存在，目标 PostgreSQL 必须为空，或仅有本程序的空表。
3. 核对命令输出的逐表行数，成功后再切换服务的数据库配置并启动。

例如服务用户和已创建的 PostgreSQL 角色都叫 `kikoeru-neo`，目标库叫 `neo_new`，通过默认端口的本机 Unix socket 认证：

```sh
sudo systemctl stop kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo db copy \
  --from /var/lib/kikoeru-neo/neo.sqlite3 \
  --to 'postgres:///neo_new?host=/run/postgresql&user=kikoeru-neo'
```

然后把 `database.url` 设置为这个目标地址。NixOS 上通过 `settings.database.url` 配置并重新激活。不要先启动目标服务再假定数据库仍为空，启动可能写入配置媒体库等记录。复制被拒绝时检查目标用途，不能自动删除一个已有数据库。

复制只搬数据库，`secrets/`、`covers/`、`lyrics/` 仍要保留。源数据库不改写，但复制开始后的新写入不会进入目标，所以迁库需要停服。全量旧库导入和 `migrate` 只生成 SQLite，之后可用 `db copy` 搬库，`import-legacy --merge` 则可以直接合并进 PostgreSQL。

## 日志和持久文件

服务器日志写到 stderr，由 systemd 或 Docker 收集。正常媒体读取只记 DEBUG，需要临时查看时设 `RUST_LOG=info,neo_server::http::access=debug`。日志可能包含用户名、路径和错误详情，对外提供前先删去自己的信息。

数据目录保存数据库、`secrets/`、`covers/`、`lyrics/` 和可能存在的 `import-report.json`。`cache/transcode/`、`cache/thumbs/`、`cache/tmp/` 可重建。配置文件、配置引用的外部密码文件和原始媒体不包含在应用备份中，完整范围见[备份与恢复](backup-restore.md)。
