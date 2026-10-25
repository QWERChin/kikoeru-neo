# 从 kikoeru-express 迁移

导入工具读取 kikoeru-express 0.6.14 到 0.6.17 的 SQLite 数据，不修改源库。全量导入适合尚未启用的新实例，合并适合已经扫描过媒体库的实例。开始前停止源服务，保留数据库、配置和封面的完整备份，并准备新实例的运行账号和磁盘空间。

## 选择导入方式

| 方式 | 目标 | 会保留什么 |
|---|---|---|
| `import-legacy` | 不存在或为空的数据目录，只生成 SQLite。 | 作品元数据、位置、用户、收藏、评价、旧播放位置和主封面，轨道由后续扫描生成。 |
| `import-legacy --merge` | 已有的 SQLite 或 PostgreSQL 库。 | 按用户和作品编号合并收藏、评价和播放进度，不导入作品元数据、封面或媒体库。 |
| `migrate` | 按目标是否已有库选择全量或合并，只接受 SQLite 配置。 | 中文交互式引导，调用同一套导入逻辑，并协助写配置和设密码。 |
| 容器迁移脚本 | 新的 Docker 数据卷。 | 从停止的旧容器复制输入，在新镜像中执行 `migrate`。 |

MySQL 源库不支持。更早版本应先在独立副本中使用固定版本的旧程序升级到支持的格式，不要拿未固定的旧镜像打开唯一的回退副本。导入不复制原始音视频文件，也不迁移歌词翻译任务。报告会列出跳过或无法匹配的记录。

## 预检源副本

以下示例假定停止旧服务后的备份位于 `/srv/kikoeru-legacy-backup`，旧媒体库叫 `Local` 和 `Cloud`。这些都是合成值，先预检，再根据自己报告中的旧媒体库名填写映射。

```sh
kikoeru-neo import-legacy --source /srv/kikoeru-legacy-backup --dry-run
```

`--source` 可以是旧数据目录、`sqlite/`、`config/`、`db.sqlite3`，或包含 `_data` 的 Docker 卷目录。预检打印找到的数据库、封面和配置位置，以及缺少的映射、字段问题或拒绝理由。加 `--json` 可以得到 JSON，但必须同时有 `--dry-run`。会拒绝导入时退出码是 2，其他失败是 1，成功是 0。

| 参数 | 何时需要 |
|---|---|
| `--db FILE`、`--covers DIR`、`--config FILE` | 覆盖旧数据库、封面目录或旧 `config.json` 的位置。 |
| `--root OLD=NEW` | 把旧 `root_folder` 名称映射到新媒体库 ID，可重复。 |
| `--map-path OLD=NEW` | 把旧配置中的路径前缀映射为当前机器或新容器中的路径，可重复，最长匹配优先。 |
| `--accept-guessed` | 检查并认可预检标为 `guessed` 的路径后使用。 |
| `--recover-journal` | 确认源服务已停止后，在工具的私有副本上恢复非空 journal 或 WAL。 |
| `--write-config FILE` | 把媒体库配置写到尚不存在的新文件，不覆盖现有文件。 |

`--root` 处理旧媒体库名字，`--map-path` 处理旧机器路径，两者不能互相替代。全量导入必须映射所有有作品的旧媒体库，旧名字拼错也会拒绝。

## 全量导入

使用有权读源副本和创建目标目录的账号运行。以下命令在子 shell 中设独立空配置，防止 NixOS 包装器或环境变量把操作指向现有 PostgreSQL。目标目录和输出配置文件都必须尚未使用：

```sh
(
  set -e
  umask 077
  import_config=$(mktemp)
  trap 'rm -f "$import_config"' EXIT
  export KIKOERU_CONFIG="$import_config"
  unset KIKOERU_DATABASE_URL

  kikoeru-neo import-legacy --dry-run \
    --source /srv/kikoeru-legacy-backup \
    --root Local=local --root Cloud=cloud \
    --map-path /usr/src/kikoeru/Local=/srv/media/works
)
```

确认预检无拒绝理由后，在同样隔离的配置下实际导入：

```sh
(
  set -e
  umask 077
  import_config=$(mktemp)
  trap 'rm -f "$import_config"' EXIT
  export KIKOERU_CONFIG="$import_config"
  unset KIKOERU_DATABASE_URL

  kikoeru-neo --data-dir /srv/kikoeru-neo-imported import-legacy \
    --source /srv/kikoeru-legacy-backup \
    --root Local=local --root Cloud=cloud \
    --map-path /usr/src/kikoeru/Local=/srv/media/works \
    --write-config /srv/kikoeru-neo-imported-roots.toml
  kikoeru-neo --data-dir /srv/kikoeru-neo-imported user set-password admin
)
```

只有导入成功后才执行设密码。示例账号 `admin` 应替换成报告中实际保留的管理员。旧密码不迁移，其他需要登录的账号也逐一设密码。

导入成功后，检查目标的 `import-report.json`，并处理生成配置中的 `# TODO` 和被注释的条目。旧 OpenList 密码不会读取，需新建 `password_file` 指向的文件，或在界面添加使用相同 ID 的媒体库。配置文件中的密码不能进入 Nix store。

把确认后的配置用于新服务，数据目录指向导入目录。NixOS 上可设置 `dataDir`，并将属主改为服务用户，目录设为 `0700`、文件设为 `0600`。不要让服务先在目标里启动，也不要把导入结果混进原有非空目录。文件配置的媒体库 ID 必须和导入映射一致。

喜欢交互操作时，在同样隔离的配置环境中改用：

```sh
kikoeru-neo --data-dir /srv/kikoeru-neo-imported migrate --source /srv/kikoeru-legacy-backup
```

引导会询问路径、媒体库 ID、OpenList 密码和账号密码。非交互运行必须加 `--yes`，它接受默认值且不设置密码。生成配置默认是目标目录下的 `config.toml`，已有该文件时写 `config.from-kikoeru.toml`，需要自己合并。

## 合并到已有库

先对目标实例[备份](backup-restore.md)。合并允许在服务运行时执行，使用服务实际的配置和数据库。以下是 NixOS 默认安装的示例，源副本必须让服务用户可读：

```sh
sudo -u kikoeru-neo kikoeru-neo import-legacy --merge --dry-run \
  --source /srv/kikoeru-legacy-backup --user old-reader=reader
sudo -u kikoeru-neo kikoeru-neo import-legacy --merge \
  --source /srv/kikoeru-legacy-backup --user old-reader=reader
```

先读完预检再运行第二条。Docker 中需把源副本只读挂载进容器，再使用同样的参数，路径填容器内位置。

默认按同名账号合并，`--user OLD=NEW` 可重复使用，且只用于合并模式。没有同名账号时创建没有密码的普通用户，之后用[账号命令](accounts.md)设密码。新库已有的收藏和评价保留，进度按更新时间选择较新记录。尚未扫描到的作品状态暂存，待新库按同一编号发现作品后再关联。

合并不创建媒体库。需要连接旧媒体目录时按[媒体库](libraries.md)另行添加，或用 `--write-config` 输出草稿。合并时的 `--root` 只控制草稿里的 ID，不决定用户状态匹配。

## 从旧容器迁移

先按 [Docker 安装](install-docker.md)构建并载入新镜像，暂不创建新卷或启动新服务。准备已停止的旧容器及独立备份。以下用 `legacy-player` 代表旧容器：

```sh
docker stop legacy-player
nix run .#kikoeru-neo-docker-migrate -- \
  --from-container legacy-player --image kikoeru-neo:0.1.0 --out ./neo
```

在源码目录运行 `nix run`，镜像标签按实际载入结果填写。脚本只检查和复制旧容器，不启动它。输入、输出都连接终端时逐项提问，否则使用 `migrate --yes` 且不设密码。

脚本默认新建 `kikoeru-neo-data`，已有同名卷就拒绝，可以用 `--volume` 换名。`--out` 默认是 `./neo`，其中不能已有 `config.toml`。`--stage-dir` 默认当前目录，需已存在且有足够磁盘空间，它临时保存数据库和封面，不要选容量不足的内存文件系统。

自动定位失败时，`--data-root` 指定旧容器内的数据目录。`--root` 和 `--map-path` 会传给引导程序，新路径指的是新容器将看到的路径。设置 `DOCKER=podman` 可选择 Podman。

成功后：

1. 阅读 `./neo/config.toml` 中的 TODO，核对媒体挂载和 OpenList 密码文件路径。
2. 把脚本打印的 Compose 片段保存为配置，结合 [Docker 安装页](install-docker.md)补上备份卷及 `KIKOERU_CONFIG=/config/config.toml`，保留它输出的实际配置文件路径和媒体挂载。脚本不自动写 `compose.yaml`。
3. 未设置密码的账号可先用下面的命令设置，名字以导入报告为准。

   ```sh
   docker run --rm -it -v kikoeru-neo-data:/data \
     kikoeru-neo:0.1.0 user set-password admin
   ```

4. 启动新 Compose，等待健康检查成功，再登录并扫描。

源媒体如果只在旧容器的可写层里，不会自动成为新服务的媒体挂载。需要另外保存媒体并挂载，导入脚本只负责旧配置、数据库和封面。导入失败时脚本尝试删除本次新建的卷，检查错误输出，不删除原有卷。

## 扫描和核对

全量导入后先扫描全部对应媒体库，轨道来自这次扫描。播放位置只有在同一作品中找到唯一匹配的完整相对路径后才能精确恢复。未匹配、歧义或用户已经在新系统修改进度时，会保留相应状态，不把旧位置随意套给第一轨。

核对本次预检、导入报告和页面上的作品、收藏、评价及进度，抽查封面和播放。收藏、已听完标记和播放历史可能重叠，不能把旧表行数直接相加当成新进度总数。`import-report.json` 是导入时的报告，不会因之后扫描而自动刷新。

保留原始备份，直到自己确认迁移结果和恢复能力。报告含自己的路径和用户数据，对外排查前先脱敏。桌面本机模式的图形导入只做合并，入口见[桌面应用](desktop.md)。
