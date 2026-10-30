# 数据模型

新库的 schema 是重新设计的，和旧 kikoeru 的表没有兼容关系。默认用 SQLite，服务器版也可以用 PostgreSQL，两边的表结构一样，见下文的“PostgreSQL”。旧数据通过一次性的导入工具搬过来，见[从旧 kikoeru 迁移数据](migration.md)。

## 设计原则

- **SQLite 分配的 ID 不复用。** `users`、`works`、`work_locations`、`tracks` 和 `tasks` 的主键都用 `AUTOINCREMENT`。token、收藏、播放进度和 URL 里都存着这些 ID，删掉一行之后，同一个 ID 不能再指向另一行。`circles` 和 `tags` 的 ID 来自 DLsite，或者按服务自己的规则编号（自定义标签从 1000000 起），不由 SQLite 分配，所以不用 `AUTOINCREMENT`。
- **作品用内部 ID，编号另存。** `works.id` 是自增的内部 ID，`works.code` 保存完整编号（如 `RJ01234567`）并且唯一。不用编号的数字部分当主键，因为 RJ、BJ、VJ 三种前缀的数字可能相同。所有 ID 都在 JavaScript 安全整数范围内，JSON 里用普通数字传输。
- **同一个编号只有一部作品，可以有多个位置。** 同一部作品出现在第二个媒体库或第二个目录里（比如本地有 mp3 版、网盘有 wav 版）时，扫描把它作为这部作品的又一个位置，所有位置的文件合并显示在同一部作品下。完全相同的文件只显示一份。
- **轨道有稳定的 ID。** 重新扫描时按原始路径匹配，同一个文件的 ID 不变，播放进度就不会丢。
- **文件路径分三种用途存。** 原始路径按字节无损保存，用来打开文件。显示路径是解码后的文字，用来展示和搜索。排序键用来自然排序。读文件时只用原始路径，不用猜过编码的显示文字。
- **缺失先标记，不直接删除。** 作品或轨道在完整扫描中确认缺失后，只设置 `missing_at`，用户状态和轨道 ID 都保留。由管理员显式清理后才物理删除。
- **用户状态和元数据分开存。** 收藏和播放进度在各自的表里，重新抓元数据不会影响它们。

## 表结构

字段类型按 SQLite 写。业务时间统一存 Unix 毫秒。

### 媒体库

```sql
CREATE TABLE roots (
  id            TEXT PRIMARY KEY,           -- 配置里的稳定标识，如 'local'
  display_name  TEXT NOT NULL,              -- 界面上显示的名字，可以改
  last_scan_at  INTEGER,
  status        TEXT NOT NULL DEFAULT 'unknown', -- 'ok' | 'unavailable' | 'unknown'
  last_scan_end_at INTEGER,                 -- 上一次扫描结束的时间，不管成败
  source_version INTEGER NOT NULL DEFAULT 0 -- 通过接口更换位置时加一，使旧媒体结果失效
);
CREATE TABLE root_settings (                -- 界面添加的媒体库，见接口合同的“媒体库管理接口”
  root_id     TEXT PRIMARY KEY REFERENCES roots(id),
  kind        TEXT NOT NULL CHECK (kind IN ('local', 'openlist', 'webdav')),
  settings    TEXT NOT NULL,                -- JSON，不含密码，密码在 secrets/roots/ 下
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
```

`roots` 的行有两个来源：`config.toml` 里的媒体库在启动时同步过来，界面添加的媒体库同时在 `root_settings` 里存一份设置。界面删除媒体库时只删 `root_settings` 的行，`roots` 的行留着，因为它的位置和轨道还指向它。改显示名字不影响任何关联，改 `id` 等于换了一个媒体库。

`root_settings.settings` 的远端设置可带 `password_version`，它与地址在同一行提交，只引用密码文件，不包含密码明文。缺少版本号的旧行继续使用未版本化的文件，因此不需要 schema 迁移。发布、回收和崩溃恢复顺序见[配置与持久状态](config.md#媒体库密码的发布与回收)。

媒体库增删改事务在 PostgreSQL 上持有事务级 advisory lock，密码回收等待同一把锁后，在 `READ COMMITTED` 事务内读取引用。SQLite 继续使用单写连接与回收检查点排序。

`source_version` 在媒体库位置更新的同一事务里递增，并清除目录摘要、探测结果和响度分析。缓存键和探测任务的去重键包含这个版本，探测及分析写回时也核对它，避免旧来源的在途任务覆盖新结果。提交确认丢失后重试更新时，新位置只要与已提交设置或当前客户端任一位置不同，仍递增版本并清除这些结果。导入的时长仍按文件变化时的规则保留，轨道 ID 和用户进度不变。

`last_scan_end_at` 是上一次扫描结束的时间，自动扫描从它开始算间隔。不从 `tasks` 表里算，是因为结束的任务会被清理。

### 作品和元数据

```sql
CREATE TABLE circles (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE vas     (id TEXT PRIMARY KEY,    name TEXT NOT NULL);   -- 沿用旧库的字符串 ID
CREATE TABLE tags    (id INTEGER PRIMARY KEY, name TEXT NOT NULL);   -- 合并后的规范标签
CREATE TABLE tag_aliases (
  alias   TEXT PRIMARY KEY,                  -- DLsite 返回的原始标签名，任何语言
  tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE
);
CREATE TABLE tag_merges (                    -- 管理员合并掉的标签，按它的 DLsite 标签 ID 抓到的名字落到合并后的标签
  from_id   INTEGER PRIMARY KEY,
  into_id   INTEGER NOT NULL,
  merged_at INTEGER NOT NULL
);

CREATE TABLE works (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  code          TEXT NOT NULL UNIQUE,         -- 'RJ01234567'
  title         TEXT NOT NULL,
  circle_id     INTEGER REFERENCES circles(id),
  release_date  TEXT,                         -- 'YYYY-MM-DD'
  nsfw          INTEGER NOT NULL DEFAULT 0,
  dl_count      INTEGER,
  price         INTEGER,
  rate_average  REAL,                         -- DLsite 的平均分，不是用户评分
  rate_count    INTEGER,
  metadata_src  TEXT NOT NULL,                -- 'dlsite' | 'asmr_one' | 'manual' | 'none'
  cover_src     TEXT NOT NULL DEFAULT 'none', -- 'dlsite' | 'asmr_one' | 'folder' | 'imported' | 'manual' | 'none'
  cover_version INTEGER NOT NULL DEFAULT 0,   -- 封面换了就加一，用于缓存
  extra         TEXT NOT NULL DEFAULT '{}',   -- JSON：排名、评分分布等不常用字段
  missing_at    INTEGER,                      -- 所有位置都缺失时才设置
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE work_locations (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  root_id       TEXT NOT NULL REFERENCES roots(id),
  rel_path      BLOB NOT NULL,                -- 作品目录的原始路径
  display_path  TEXT NOT NULL,
  listing_hash  TEXT,                         -- 上次完整列出时目录内容的摘要，见“扫描”
  listed_at     INTEGER,
  missing_at    INTEGER,
  UNIQUE (root_id, rel_path)
);
CREATE INDEX work_locations_work ON work_locations (work_id);

CREATE TABLE work_vas  (work_id INTEGER REFERENCES works(id) ON DELETE CASCADE,
                        va_id   TEXT    REFERENCES vas(id)   ON DELETE CASCADE,
                        PRIMARY KEY (work_id, va_id));
CREATE TABLE work_tags (work_id INTEGER REFERENCES works(id) ON DELETE CASCADE,
                        tag_id  INTEGER REFERENCES tags(id)  ON DELETE CASCADE,
                        PRIMARY KEY (work_id, tag_id));
```

`metadata_src` 的转换规则：

| 当前值 | 事件 | 新值 |
|---|---|---|
| （新作品） | 抓取成功 | `dlsite` |
| （新作品） | 抓取失败 | `none`，标题用目录名 |
| `none` | 用户点“重新抓取”且成功 | `dlsite` |
| `dlsite` | 普通重新抓取 | `dlsite`，字段被覆盖 |
| 任意 | 管理员手动编辑 | `manual` |
| `manual` | 普通重新抓取 | 不变，跳过 |
| `manual` | 管理员选“强制覆盖” | `dlsite` |
| `none` 或 `asmr_one` | DLsite 抓取失败，备用来源（`metadata.fallback`）抓取成功 | `asmr_one` |
| `asmr_one` | 普通重新抓取，DLsite 抓取成功 | `dlsite` |
| `manual` | 强制覆盖，DLsite 抓取失败、备用来源成功 | `asmr_one` |

封面的规则类似：`cover_src` 是 `imported` 或 `manual` 的封面，普通的元数据刷新不会覆盖，只有强制覆盖才会。`dlsite` 和 `asmr_one` 的封面普通刷新照常替换，`asmr_one` 表示封面来自备用来源，可能是元数据来自 DLsite、只有封面从 asmr.one 补上的作品。`folder` 是扫描从作品文件夹里挑的图片，只在作品没有封面时才写，来源一给出封面就被普通刷新替换，见[扫描与元数据](scanning.md)的“封面”。

### 轨道

```sql
CREATE TABLE tracks (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  location_id   INTEGER NOT NULL REFERENCES work_locations(id) ON DELETE CASCADE,
  rel_path      BLOB NOT NULL,          -- 相对于这个位置的作品目录的原始路径
  display_path  TEXT NOT NULL,
  sort_key      TEXT NOT NULL,
  kind          TEXT NOT NULL,          -- 'audio' | 'video' | 'subtitle' | 'image' | 'text' | 'other'
  size          INTEGER,
  mtime         INTEGER,                -- 远端拿不到可靠时间时为 NULL
  duration_ms   INTEGER,                -- ffprobe 之后才有；导入的时长作为提示先填
  duration_src  TEXT,                   -- 'probe' | 'imported'
  probe         TEXT,                   -- JSON：codec、采样率、声道、分辨率
  duplicate_of  INTEGER REFERENCES tracks(id), -- 和另一个位置的文件完全相同时指向保留的那条
  missing_at    INTEGER,
  UNIQUE (location_id, rel_path)
);
CREATE INDEX tracks_work_kind ON tracks (work_id, kind);

-- analysis 任务测出的响度（见媒体处理与播放的“响度和波形”）。size 和 mtime
-- 是测量时源文件的值，和 tracks 里的不一样就作废。波形放在缓存里，不在这里。
CREATE TABLE track_analysis (
  track_id        INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
  size            INTEGER,
  mtime           INTEGER,
  integrated_lufs REAL,                 -- 测不出（静音）时为 NULL
  true_peak_dbtp  REAL,                 -- 全是静音时为 NULL
  analyzed_at     INTEGER NOT NULL
);
```

文件夹结构不单独建表。API 按确定的顺序返回扁平的轨道列表，前端按 `display_path` 拆出目录树。

### 多个位置怎么合并

- 一部作品的所有位置的文件都放在这部作品下，前端看到的是一棵合并后的目录树。
- 两个位置里相对路径相同、大小也相同的文件视为同一个文件，只显示一份，另一条的 `duplicate_of` 指向保留的那条。保留的优先顺序：本地媒体库优先于 OpenList 和 WebDAV，其余按媒体库的顺序。
- 路径相同但大小不同，或者路径不同的文件都显示。比如一个位置有 `mp3/`，另一个位置有 `wav/`，详情页里就同时看到这两个文件夹。
- 两个位置都有、但顶层目录名不同的情况（比如 `mp3版/` 和 `MP3/`）当前不自动识别为同一个文件夹，按两个文件夹显示。
- 保留的那条所在的位置缺失时，下一次扫描把 `duplicate_of` 指向它的另一条提升为保留的那条，播放进度里的 `track_id` 一并改过去。
- 扫描隐藏一条时，指向它的播放进度改指保留的那条。正在播放的队列不会跟着扫描更新，之后的进度上报和播完上报仍可能带着被隐藏的那条，后端在写入的同一条语句里把它换成保留的那条，判断“分组最后一轨”时也按保留的那条算。否则详情页找不到这条轨道，续播会退回分组开头。扫描进行中，保留的那条可能已经换了大小或被标记缺失，而去重还没重算，所以只有两条仍然路径相同、大小相同、保留的那条也还在时才换，否则进度留在播放器实际播的那条上。

作品详情的摘要、轨道和其他组成部分在同一个只读事务里读取，整个响应来自同一个快照。扫描在读取期间提交去重和进度迁移时，详情不能混用迁移前的进度和迁移后的轨道列表，否则续播仍会找不到进度指向的轨道。

`size` 或 `mtime` 变了，就清空 `duration_ms`（除非来自导入）、`probe`，让后台任务重新探测，同时让依赖这个文件的缓存失效。`mtime` 为 `NULL` 的远端文件，缓存按保守的有效期失效，并提供手动刷新。

### 用户和用户状态

```sql
CREATE TABLE users (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  name             TEXT NOT NULL UNIQUE,
  password_hash    TEXT,                -- argon2id；NULL 表示必须先由本地命令设置密码
  is_admin         INTEGER NOT NULL DEFAULT 0,
  token_version    INTEGER NOT NULL DEFAULT 0, -- 新建用户时取随机初始值，见“认证”
  created_at       INTEGER NOT NULL
);

CREATE TABLE favorites (
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE play_progress (
  user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  track_id      INTEGER REFERENCES tracks(id),
  position_ms   INTEGER NOT NULL DEFAULT 0,
  group_path    TEXT,                 -- 用户选定的播放分组（目录），用于判断播完
  finished_at   INTEGER,              -- 非空表示“听过”
  replaying     INTEGER NOT NULL DEFAULT 0, -- 听过之后又开始播放
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE progress_report_cursors (
  user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  session_id    TEXT NOT NULL,
  last_sequence INTEGER NOT NULL,
  PRIMARY KEY (user_id, work_id, session_id)
);

CREATE TABLE legacy_reviews (
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  rating       INTEGER,
  review_text  TEXT,
  progress     TEXT,                  -- 旧的 marked/listening/listened/replay/postponed
  PRIMARY KEY (user_id, work_id)
);
CREATE TABLE reviews (                 -- 用户自己的评分和短评，见接口合同的“评分和短评”
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  rating      INTEGER CHECK (rating BETWEEN 1 AND 5),
  text        TEXT,
  updated_at  INTEGER,                -- 导入的旧记录没有时间时为空
  PRIMARY KEY (user_id, work_id),
  CHECK (rating IS NOT NULL OR text IS NOT NULL)
);
CREATE TABLE user_settings (           -- GET/PUT /api/me/settings，没有行时是默认值
  user_id            INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  normalize_loudness INTEGER NOT NULL DEFAULT 0,
  target_lufs        REAL NOT NULL DEFAULT -16,
  accent             TEXT,           -- 接口合同的 Accent，NULL 是默认的蓝色
  updated_at         INTEGER NOT NULL,
  seek_back_seconds    INTEGER NOT NULL DEFAULT 15,  -- 后退按钮的秒数，1 到 99
  seek_forward_seconds INTEGER NOT NULL DEFAULT 30   -- 前进按钮的秒数，1 到 99
);
```

`legacy_reviews` 是旧 `t_review` 的原样存档，导入之后不再修改。界面读写的评分和短评在 `reviews`：导入时把旧记录里有效的评分和短评复制过去，之后只改 `reviews`。加上 `reviews` 的数据库迁移按同样的规则从已经导入的 `legacy_reviews` 补一遍，这些行的 `updated_at` 为空，因为 `legacy_reviews` 没有保存时间。分开存是因为旧记录里大多只有 `marked` 这样的收听状态，它们已经变成了收藏和进度，不该算作评价。

`track_id` 指向的轨道被标记缺失时，进度保留不变。轨道被物理清理时，清理命令先把相关进度的 `track_id` 置空。

### 服务器设置

```sql
CREATE TABLE server_settings (         -- GET/PUT /api/admin/settings，没有行时是默认值
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  metadata_proxy  TEXT,                -- 整理过的代理地址，NULL 表示按环境变量
  updated_at      INTEGER NOT NULL
);
```

整个服务共用、由管理员在设置页修改的设置，最多一行。每个设置一列，而不是键值对，这样每一项都有自己的类型和约束，和 `user_settings` 一样。代理地址在写入前已经按[接口合同](contract.md)的“服务器设置”检查和整理过。

### 导入辅助表

```sql
CREATE TABLE import_pending (
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  source       TEXT NOT NULL,         -- JSON：旧队列项、原始秒数、旧行标识
  durations    TEXT,                  -- JSON：旧 memo 里的路径到时长
  state        TEXT NOT NULL,         -- 'pending' | 'matched' | 'ambiguous' | 'unmatched' | 'superseded'
  PRIMARY KEY (user_id, work_id)
);
```

`import_pending` 只服务于迁移：它保存把旧进度对到新轨道所需的材料，让扫描失败或服务重启之后还能继续匹配。详细规则见[迁移](migration.md)。即使所有记录已经处理，也必须保留这张表。扫描和数据库复制仍会访问它，只有正式的数据库迁移才能退役它。

```sql
CREATE TABLE legacy_user_state (
  user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  code               TEXT NOT NULL,        -- 作品编号，作品可能还没扫描到
  favorite_at        INTEGER,              -- 非空表示旧库里收藏了
  has_review         INTEGER NOT NULL DEFAULT 0,
  rating             INTEGER,
  review_text        TEXT,
  review_progress    TEXT,
  progress_at        INTEGER,              -- 旧进度的更新时间，空表示没有进度
  position_ms        INTEGER,
  finished_at        INTEGER,
  pending_source     TEXT,                 -- 进度的 import_pending 材料
  pending_durations  TEXT,
  pending_state      TEXT,
  review_at          INTEGER,              -- 旧评价的时间，挂到作品上时写进 reviews.updated_at
  PRIMARY KEY (user_id, code)
);
CREATE INDEX legacy_user_state_code ON legacy_user_state (code);
```

`legacy_user_state` 保存合并到已有库时还找不到作品的旧用户数据，按作品编号等扫描。扫描新建这个编号的作品时，把这些行挂到作品上并删掉，规则见[迁移](migration.md)的“合并到已有的库”。

### 后台任务

```sql
CREATE TABLE tasks (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL,       -- 'scan' | 'probe' | 'transcode' | 'metadata' | 'analysis'
  dedupe_key  TEXT,                -- 同一个键同时只能有一个活动任务
  state       TEXT NOT NULL,       -- 'queued' | 'running' | 'done' | 'failed' | 'cancelled' | 'interrupted'
  payload     TEXT NOT NULL,       -- JSON
  result      TEXT,                -- JSON：成功时比如可播放的 URL
  progress    REAL,
  message     TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  queue_rank  INTEGER              -- 移到最前的扫描排在前面，见下文
);
CREATE UNIQUE INDEX tasks_active_dedupe ON tasks (dedupe_key)
  WHERE state IN ('queued', 'running');
```

状态转换只有这几条：`queued → running → done | failed | cancelled`，`queued → cancelled`。服务启动时，上次留下的 `running` 改成 `interrupted`，`queued` 重新交给调度器继续排队。不把丢失的任务当成成功。

PostgreSQL 入队事务遇到连接或协议错误，无法确认是否提交时，调度器保留插入返回的任务 ID，并为原任务登记运行器。运行器沿用启动任务时的条件更新，只有数据库中仍有活动行才执行任务，数据库暂时不可用时继续重试，已回滚或已取消的任务不会执行。提交方仍收到原数据库错误，后续去重不会创建第二个运行器，没有去重键的任务也能恢复，不需要等待重启。SQLite 的入队行为不变。

服务先成功绑定监听端口，再恢复任务。端口被占用时，排队任务保持原样，不访问媒体库。恢复过程中发生错误时，服务先关闭调度器，在关闭期限内等待已启动的任务清理，再返回启动错误。超时后的清理和实例锁仍由后台任务持有，规则见[媒体处理与播放](media.md)的“子进程”。

重新排队按 `COALESCE(queue_rank, 0)`、`id` 的顺序。只有扫描会写 `queue_rank`：管理员把一个排队中的扫描移到最前时，它的值改成排队中的扫描里最小的值减一。所以最后移动的排在最前，没移动过的按提交顺序排在后面，和调度器内存里的顺序一致，重启不会把管理员排好的顺序打乱。

## 收听状态怎么算

封面上的图标都由上面的表算出来，用户只手动维护收藏：

| 情况 | 封面图标 | 打开作品时 |
|---|---|---|
| 没有播放记录 | 无 | 从用户选的轨道开始 |
| 有记录，`finished_at` 为空，轨道能定位 | 绿色进度圈 | 恢复到当前轨道和位置 |
| 有导入的旧记录，轨道还没对上 | 绿色进度圈，详情页提示“无法精确恢复” | 不把旧位置套到任意一条轨道上 |
| `finished_at` 非空，`replaying = 0` | 灰色对勾 | 有有效位置时仍可恢复 |
| `finished_at` 非空，`replaying = 1` | 灰色对勾加绿色进度圈 | 恢复到当前轨道和位置 |
| 收藏 | 红心，和上面任意一种叠加 | 不影响播放状态 |

API 里的 `Progress` 按这张表算：`finished` 就是 `finished_at` 非空。`replaying` 只在 `finished` 为真时才可能为真，数据库里 `finished_at` 为空而 `replaying = 1` 的行按没有重听处理。`resumable` 要求 `track_id` 非空，并且这条轨道没有被标记缺失，因为缺失的轨道没法恢复播放。`track_duration_ms` 取当前轨道的 `duration_ms`。

转换规则：

- 在听过的作品上开始播放，设置 `replaying = 1`，`finished_at` 不变。
- 播完选定分组的最后一轨（见下文），设置 `finished_at` 为当前时间，`replaying = 0`。
- 清除进度，删除 `play_progress` 中的整行，但保留会话序号记录。

### 进度报告的会话序号

`progress_report_cursors` 保存每个用户、作品和播放器会话已成功接受的最大序号。会话标识和序号的格式由 HTTP 边界按[接口合同](contract.md)校验，存储层接收借用参数 `ReportInput`，不重复校验。迁移 0014 只建表，不为已有进度回填会话。

每次报告在 `retry_busy` 的一次尝试内开启事务。有序报告先执行条件 UPSERT，只有首次写入或序号比水位更大时才返回行。没有返回行就结束事务，不执行进度 UPSERT，连 `updated_at`、完成状态和重听状态也保持不变。接受报告时，推进水位和写入进度在同一连接、同一事务中提交，任一步失败都会回滚。发生可重试的写冲突时，从新事务重新比较水位，复用原始输入、序号和服务器时间。

其他会话的报告不会替换已有会话的水位，因此 `A2 → B1 → A1` 和 `A2 → B1 → A2副本` 都保留 B1 的进度。不同会话中尚未过期的报告按实际成功写入顺序覆盖，不比较它们的序号。序号更大的报告可以写入更小的位置，允许用户往回拖动。`report_order` 为 `null` 或缺失的旧格式报告走同一事务路径，但不访问水位表，继续直接覆盖进度。旧格式报告本身没有乱序保护。

水位只引用用户和作品，不引用进度行或轨道。清除进度、标记完成和删除轨道都不删除水位，已知旧包不能重建已清除的进度，重复包也不能让已完成的作品进入重听。更大的序号仍能记录新的播放。这里不为 PUT、clear 和 finish 建立统一顺序，尚未被同会话更大序号覆盖的报告仍可在 clear 或 finish 后首次写入。

每个用户、作品、会话组合占一行，刷新页面和新的账号周期会继续累积水位。当前只随用户或作品删除级联清理，不设置 TTL、额外索引或会话限额。数据库复制也必须保留这些行，否则切换后端后会重新接受旧包。

### 什么算“播完”

播放队列由用户选定的目录分组生成，比如 `mp3/SE有/`，按自然排序排列，不把不同格式或不同版本的目录拼在一起。前端在这个分组最后一轨触发 `ended` 事件时，调用 `POST /api/me/progress/{work_id}/finish`，带上 `track_id` 和 `group_path`，后端确认这条轨道属于这部作品、确实是这个分组的最后一轨。后端不从普通的进度上报推断是否播完。“最后一轨”指这个目录下直接包含的音频和视频里按 `sort_key` 排在最后的一条，子目录里的文件、已标记缺失的文件和 `duplicate_of` 不为空的重复文件都不算，因为播放器的队列里也没有它们。`group_path` 是相对作品目录的路径，作品顶层是空字符串，末尾多一个 `/` 也接受。标记播完时保留已有的位置，如果换了轨道则位置归零。

刷新页面后，播放器从 `play_progress` 恢复同一个分组。分组就是轨道所在的目录，所以只要轨道能定位，前端就用这条轨道的目录作为分组，导入的旧进度 `group_path` 为空时也一样。轨道定位不了时不恢复，也不自动标记播完。作品详情页的“标记为听过”按钮走同一个接口，带 `manual=true` 作为手动修正，不要求当前轨道是分组最后一轨。

## SQLite 设置

- 新库放在本地磁盘的数据目录里，用 WAL 日志模式。数据库不放在网络文件系统上。
- 每个连接打开 `foreign_keys`，`busy_timeout` 设为 5 秒。
- 一个只有 1 个连接的写连接池，所有写事务都经过同一个入口。另一个 4 个连接的只读池给查询用。
- 写事务要短。扫描按批次提交，网络请求和 ffmpeg 运行期间不持有写事务。
- `SQLITE_BUSY` 只对完整、可以安全重复的事务重试，最多 3 次，每次多等 50 毫秒。`busy_timeout` 和单一写连接之外仍然会遇到它：别的进程（比如 `user` 命令）持锁超过 5 秒，或者一个事务先读、别的进程随后写入，这个事务再写时会立刻得到 `SQLITE_BUSY_SNAPSHOT`。重试由 `neo_core::db::retry_busy` 负责，每次调用必须是一个从头到尾的事务，并且对数据库以外没有副作用。目前任务表、播放进度和评分短评的写入走这个入口。

查询优先用 SQLx 的 `query!` 宏，离线元数据（`.sqlx/`）提交进仓库，CI 用 `cargo sqlx prepare --check` 确认没有过期。搜索这类拼接条件的动态 SQL 用 `QueryBuilder`，它不在编译期检查之内，靠集成测试覆盖。

## PostgreSQL

不配置时数据库是数据目录里的 SQLite 文件。服务器版也可以改用 PostgreSQL，桌面版只用 SQLite。两边的表、列、主键、唯一约束、索引和外键完全一样，同一条 SQL 在两边读出的 Rust 类型也一样。为此 PostgreSQL 的建表语句按下面的规则对应 SQLite：

| SQLite | PostgreSQL | 原因 |
|---|---|---|
| `INTEGER` | `BIGINT` | SQLite 的整数总是读成 `i64`，PostgreSQL 的 `INTEGER` 只有 32 位 |
| 存标志的 `INTEGER`（`nsfw`、`is_admin`、`replaying`、`has_review`、`normalize_loudness`） | `BOOLEAN` | 查询里用 `AS "x: bool"` 读它们，PostgreSQL 不能把整数当布尔值读 |
| `INTEGER PRIMARY KEY AUTOINCREMENT` | `BIGINT GENERATED BY DEFAULT AS IDENTITY` | 导入时要按旧 ID 写入，`BY DEFAULT` 允许显式写 ID，写完之后要把序列推到最大 ID 之后 |
| `TEXT` | `TEXT COLLATE "C"` | 按字节比较和排序，和 SQLite 默认的 `BINARY` 一样，不受数据库 locale 影响 |
| `REAL` | `DOUBLE PRECISION` | 都读成 `f64` |
| `BLOB` | `BYTEA` | 都读成 `Vec<u8>` |

SQL 只用两边都接受的写法：占位符写成 `$1`，标志用 `TRUE` 和 `FALSE`，可空值的相等比较用 `IS NOT DISTINCT FROM`，覆盖写入用 `ON CONFLICT ... DO UPDATE`，新行的 ID 用 `RETURNING` 取。SQLite 的 `LIKE` 不区分 ASCII 大小写，PostgreSQL 区分，所以搜索时列一侧先 `lower()`。SQLite 的 `lower()` 和 PostgreSQL 在 `COLLATE "C"` 下的 `lower()` 都只转 ASCII 字母，搜索词在 Rust 里用 `to_ascii_lowercase()` 按同样的规则转，不写 `lower($1)`：参数没有列的排序规则，PostgreSQL 会按数据库默认的排序规则转，默认值不是 C 时连全角字母也会转，原样输入的标题反而搜不到。PostgreSQL 要能推断出每个参数的类型，参数第一次出现不能是 `$1 IS NULL` 这种形式。

PostgreSQL 的迁移从版本 9 开始，`0009_baseline.sql` 一次建出 SQLite 走完 0001 到 0009 之后的表结构。之后每个迁移在 `migrations/sqlite/` 和 `migrations/postgres/` 各写一份，用同一个编号，所以同一个版本号在两边代表同一个表结构。`neo-db` 的 `pg_schema` 测试分别跑两边的迁移，比较表、列的顺序、类型、可空、默认值、主键、唯一约束、索引和外键，只改了一边就会失败。部分索引只比较是不是部分索引，条件在两边写法不同，所以另有一个测试在两边执行同样的插入，确认 `tasks_active_dedupe` 的效果一样。`CHECK` 约束不在比较之内。

PostgreSQL 上的连接和 SQLite 的安排一样：一个只有 1 个连接的写连接池，一个 4 个连接的只读池。写连接把默认隔离级别设成 `SERIALIZABLE`，所以先读后写的事务在两边得到同样的保证，比如“最后一个管理员不能删”这类检查不会和别的写入交错。只读池设成 `REPEATABLE READ, READ ONLY`，一个读事务里的几条查询看到同一个快照，例如数据库复制在一个只读快照中读取多张关联表，不会把不同提交时点的数据拼在一起。和 SQLite 不同，`db copy`、`user` 命令这类别的进程也可能同时写，事务提交时可能因为冲突失败，SQLSTATE 是 `40001` 或 `40P01`。`neo_core::db::is_busy` 把它们和 `SQLITE_BUSY` 同样对待，所以走 `retry_busy` 的写入在两边都会重试。

这些测试需要一个用完即删的 PostgreSQL 17 或更新的版本。`KIKOERU_TEST_PG` 指向它，用户要有建库权限，每个测试建一个自己的库。测试库的默认排序规则是内置的 `C.UTF-8`，它和真实服务器常用的 locale 一样会转换非 ASCII 字母的大小写，依赖数据库默认排序规则而不是列的 `COLLATE "C"` 的 SQL 在测试里就会出错。`neo-db` 和 `neo-scanner` 的数据库测试通过 `neo_db::testing::open()` 拿到数据库，设置了 `KIKOERU_TEST_BACKEND=postgres` 时它在 `KIKOERU_TEST_PG` 上建库，否则在临时目录里建 SQLite，所以这些测试可以切换后端。`nix flake check` 的 `rust-test-pg` 在构建沙箱里启动一个只监听 Unix socket 的服务器，只运行 `db`、`legacy_state`、`pg_schema`、`db_copy`、`scan` 和 `metadata` 目标，不包含全部可切换后端的测试，覆盖缺口见[测试与验证](testing.md)。本地可以用开发环境里的 `initdb` 和 `pg_ctl` 起一个，再运行 `KIKOERU_TEST_BACKEND=postgres cargo test -p neo-db -p neo-scanner -- --include-ignored`。只有 SQLite 才有的测试（比如 `PRAGMA` 设置、`VACUUM INTO`）直接用 `Db::open_sqlite`，不随这个变量切换。

### 存储层的代码结构

共享业务查询写在 `crates/neo-db/queries/` 里，每个领域一个文件，一个函数通常就是一条语句。迁移、数据库复制、后端专用语句和测试是明确的例外。扫描、导入和 HTTP 处理只调用这些函数，自己不写 SQL，所以换数据库时不用改它们。

- `crates/neo-db/queries/` 不是一个 crate。`neo-db-sqlite` 和 `neo-db-postgres` 都用 `#[path]` 把它编译一遍，两边的 `query!` 各自对照自己的数据库检查。查询里的连接池和事务写成 `crate::Pool`、`crate::Tx`，由各自的 crate 定义。`neo-db-postgres` 的 `sqlx.toml` 让它从 `PG_DATABASE_URL` 读数据库地址，所以一次编译里两个 crate 能各连各的库。
- 同一段 SQL 在两边读出的 Rust 类型必须一样。PostgreSQL 没有无符号整数，绑定 `u8`、`u32` 之前先转成 `i64`。`EXISTS (...)` 这类表达式在 PostgreSQL 上算作可空，要写成 `AS "x!: bool"`。
- 查询用到的行类型、参数类型和错误类型放在 `neo_core::db` 的同名模块里，两个后端共用同一份，调用方拿到的类型和后端无关。
- `neo-db` 是调用方唯一用到的入口。`Db` 和 `Tx` 是枚举，每个变体是一种后端。`dispatch!` 为每条查询生成一个同名函数，参数从连接池换成 `&Db`，事务内的查询换成 `&mut Tx`，按 `Db` 的变体调用对应后端。`read` 走只读池，`write` 走写连接，`tx` 走调用方的事务。
- 加一条查询要做三件事：在 `queries/` 的对应文件里写函数，类型放进 `neo_core::db`，在 `neo-db` 的 `dispatch!` 里加一行。
- 只有 SQLite 有的语句（`VACUUM INTO`、`PRAGMA integrity_check`）不进 `queries/`，调用方通过 `Db::sqlite()` 拿到 SQLite 的连接池来执行，测试里直接写 SQL 时也这样做。

### 在两种数据库之间复制

`neo_core::db::copy` 按 `TABLES` 里的顺序逐表复制，父表在前，所以每个外键在写入时都能找到它指向的行。`tracks.duplicate_of` 可能指向 ID 更大的行，复制时先写成 NULL，整张表写完后再补上。每张表按主键顺序流式读出，每 500 行写一次，最宽的表一次绑定 7000 个值，低于 SQLite 的 32766 和 PostgreSQL 的 65535。源在 SQLite 上是一个读事务，在 PostgreSQL 上是 `REPEATABLE READ READ ONLY` 事务，两者都只看到同一时刻的快照。目标的全部写入在一个事务里，每张表写完后用 `COUNT(*)` 核对行数。写入 PostgreSQL 后把每个 identity 序列推到最大 ID 之后。`TABLES` 由单元测试对照迁移后的 SQLite 表结构检查，加了列而忘了改它时测试失败。

## 迁移规则

schema 用 `sqlx migrate` 管理，SQLite 的迁移文件放在 `crates/neo-core/migrations/sqlite/`，PostgreSQL 的放在 `crates/neo-core/migrations/postgres/`。启动时如果迁移失败，服务直接退出并报错，不会带着半升级的库继续运行。这一点和旧 kikoeru 不同：旧代码在迁移出错时只打日志，然后照常启动。
