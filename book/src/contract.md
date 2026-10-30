# 接口合同

本章定义当前前后端之间的请求、响应、字段类型、权限和错误码。[API 设计](api.md)只解释设计与调用流程，不重复字段定义。接口按资源和用途分组。

仓库的 `contract/` 目录保存合成的请求和响应样本，二进制与无响应体的接口通过相应测试验证。前后端共同使用这些样本：

- **前端**用样本驱动假数据模式和界面测试。
- **后端**为每个样本写一个合同测试：用同样的数据构造 DTO，序列化后比较完整 JSON 值，并验证反序列化，字段、类型、空值和具体值均须一致。

## 怎么改这份合同

1. 需要改接口的一方，先单独提交一个只改这一章和 `contract/` 样本的改动，提交标题用 `docs: update API contract for ...`。
2. 后端在后续实现提交中更新 `neo-core::dto`，让合同测试通过。
3. 前端随后使用更新的生成类型修改页面。合同、后端和前端提交可以在同一个拉取请求中一起验证，但合同提交不夹带实现。

不要在合同之外私下约定字段。TypeScript 类型由后端用 ts-rs 从 DTO 生成，前端不手写。

## 通用规则

| 规则 | 内容 |
|---|---|
| 字段命名 | `snake_case` |
| ID | 数值 ID 是不超过 2^53 − 1 的正整数，使用 JSON number。声优、媒体库和进度会话等字符串 ID 以各类型定义为准 |
| 时间 | Unix 毫秒，JSON number |
| 响应可空字段 | 一定出现，无值时为 `null`，不省略字段 |
| 请求缺省 | 是否可省略及缺省值由各请求定义，不从响应的空值规则推导 |
| 布尔值 | `true` / `false`，不用 0 和 1 |
| 枚举 | 小写字符串 |
| 成功但没有内容 | HTTP 204，没有响应体 |
| 压缩 | 请求带 `Accept-Encoding: gzip` 时，1 KiB 以上的 JSON 和纯文本（`text/vtt`、`text/plain`，比如字幕）用 gzip 压缩，回答带 `Vary: Accept-Encoding`。音频、视频、封面、打包下载和任务事件流不压缩，带 `Accept-Ranges` 的回答也不压缩 |

`ProgressRequest.report_order` 有一个仅用于旧客户端的兼容例外：服务器接受缺失的字段，并按 `null` 处理。新格式仍必须包含此字段，不把它定义成可省略字段。这个例外不适用于非空排序对象内部的成员。

### 错误

所有错误响应都是这个形状，见 `contract/m1/error-unauthorized.json`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `error.code` | string | 错误码，取值见下表及各接口的错误说明 |
| `error.message` | string | 给人看的说明，前端可以直接显示，但不要拿来做判断 |

除了下文各接口列出的错误，任何接口都可能因为服务自己出错返回 500 `internal_error`，前端把它当作一般的失败显示。

不存在的 `/api` 路径返回 404 `not_found`。路径存在但请求方法不对时返回 405 `invalid_request`。两种情况都是上面的 JSON 形状，不是 HTML 页面。

| HTTP | `code` | 什么时候 |
|---|---|---|
| 400 | `invalid_request` | 参数格式不对 |
| 401 | `unauthorized` | 没有 token、token 过期或已失效 |
| 401 | `invalid_credentials` | 用户名或密码不对 |
| 401 | `setup_required` | 还没有任何管理员，见[认证](auth.md) |
| 403 | `forbidden` | 普通用户访问管理接口 |
| 404 | `not_found` | 本服务里没有这个作品、轨道或任务，或者没有这个接口 |
| 404 | `not_applicable` | 轨道存在，但这个接口不适用于它的类型，比如对视频请求响度或波形 |
| 405 | `invalid_request` | 接口存在，但不支持这个请求方法 |
| 409 | `media_not_ready` | 请求了还没准备好的可播放文件 |
| 409 | `scan_running` | 有扫描正在进行时请求清理缺失项，或者删除正在扫描的媒体库 |
| 409 | `root_exists` | 添加媒体库时这个 `id` 已经被占用 |
| 409 | `root_read_only` | 想通过接口修改或删除 `config.toml` 里的媒体库 |
| 410 | `missing` | 作品或轨道已被标记为缺失 |
| 413 | `too_large` | 结果超过上限（比如单作品轨道数超过 5000） |
| 429 | `queue_full` | 转码队列已满 |
| 429 | `too_many_attempts` | 登录或修改密码时密码错误次数太多，规则见[认证](auth.md) |
| 500 | `internal_error` | 服务自己出错，比如数据库查询失败。原因只写进日志，不放进响应 |
| 502 | `upstream_error` | 远端存储返回业务错误，或上游请求失败 |
| 503 | `not_ready` | 服务还没就绪 |
| 503 | `storage_unavailable` | 媒体库连不上、登录失败或超时 |
| 507 | `insufficient_storage` | 磁盘空间低于 `cache.min_free_bytes` |

上游的认证错误不直接表示本服务的登录失效，按存储错误分类转换，见[存储后端](storage.md)。

### 分页

列表接口的响应都是这个形状：

| 字段 | 类型 | 说明 |
|---|---|---|
| `items` | array | 当前页的条目 |
| `page` | number | 从 1 开始 |
| `page_size` | number | 实际使用的每页条数 |
| `total` | number | 满足条件的总条数 |

## 类型

### `User`

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | |
| `name` | string | |
| `is_admin` | boolean | |

### `CircleRef`

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | |
| `name` | string | |

### `Progress`

| 字段 | 类型 | 说明 |
|---|---|---|
| `track_id` | number \| null | 导入的旧进度还没对上轨道时为 `null` |
| `position_ms` | number | 在当前轨道里的位置 |
| `track_duration_ms` | number \| null | 当前轨道的时长，不知道时为 `null`，前端不画进度条 |
| `finished` | boolean | 是否听过 |
| `replaying` | boolean | 听过之后是否又在播放 |
| `resumable` | boolean | 能否精确恢复。`track_id` 为 `null` 时一定是 `false` |
| `group_path` | string \| null | 上次播放的目录分组（相对于作品目录，比如 `mp3/SE有`，作品根目录是空字符串）。恢复播放队列用。导入的旧进度和不知道分组时为 `null` |

封面图标的规则见[数据模型](data-model.md)的“收听状态怎么算”。

### `ProgressReportOrder`

| 字段 | 类型 | 说明 |
|---|---|---|
| `session_id` | string | 恰好 32 个小写 ASCII 十六进制字符，来自随机 128 bit 标识。它标识一次播放器账号周期，不用于认证。 |
| `sequence` | number | JSON 整数，范围为 `1..=9007199254740991`。同一会话内严格递增，允许跳号，不得回绕。 |

两个字段都必须出现，且不能为 `null`。同一逻辑报告的重传必须复用完整请求体及序号，不同逻辑报告不得复用同一序号。页面隐藏或关闭时立即发送的报告与它的排队副本属于同一逻辑报告。

新页面、新播放器上报实例及账号重置开始新会话。切轨、切作品、暂停恢复、隐藏恢复和同一页面从 BFCache 恢复都不重置会话或序号。同账号 token 更新后的重试保留原请求体和排序身份。

### `ProgressRequest`

| 字段 | 类型 | 说明 |
|---|---|---|
| `track_id` | number | 当前轨道的 ID，必须属于请求路径指定的作品。 |
| `position_ms` | number | 在当前轨道里的位置，为 `0..=9007199254740991` 范围内的 JSON 整数。 |
| `group_path` | string | 相对于作品目录的分组路径，作品根目录为空字符串，最多 4096 字节。 |
| `report_order` | `ProgressReportOrder` \| null | 新格式必须包含此字段。新版播放器发送非空对象，显式 `null` 使用旧覆盖规则。仅为兼容旧客户端，服务器接受字段缺失并按 `null` 处理。 |

排序字段只用于请求，`Progress` 响应保持不变，不包含会话标识或序号。

### `WorkSummary`

作品列表里的一项。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | 内部 ID |
| `code` | string | 完整编号，如 `RJ01234567` |
| `title` | string | |
| `circle` | `CircleRef` \| null | |
| `release_date` | string \| null | `YYYY-MM-DD` |
| `nsfw` | boolean | |
| `rate_average` | number \| null | DLsite 平均分，0 到 5 |
| `rate_count` | number \| null | DLsite 评价人数 |
| `has_cover` | boolean | `false` 时前端显示占位图，不请求封面 |
| `cover_version` | number | 拼进封面 URL，封面换了它就变 |
| `missing` | boolean | 所有位置都缺失。默认的列表里不会出现 `true` |
| `favorite` | boolean | 当前用户是否收藏 |
| `progress` | `Progress` \| null | 当前用户的播放进度，没有记录时为 `null` |
| `my_rating` | number \| null | 当前用户给的评分，1 到 5 的整数，没评过时为 `null`，见“评分和短评” |

### `VaRef` 和 `TagRef`

| 类型 | 字段 |
|---|---|
| `VaRef` | `id`（string，沿用旧库的字符串 ID）、`name`（string） |
| `TagRef` | `id`（number）、`name`（string，规范标签名） |

### `TrackItem`

作品详情里的一条文件。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | 轨道 ID |
| `path` | string | 显示路径，相对于作品目录，用 `/` 分隔，比如 `mp3/SE有/Tr01.mp3`。前端按它拆目录树 |
| `kind` | string | `audio`、`video`、`subtitle`、`image`、`text`、`other` |
| `size` | number \| null | 字节数，远端不知道时为 `null` |
| `duration_ms` | number \| null | 时长，还没探测时为 `null` |
| `has_subtitle` | boolean | 同目录下有同名的 lrc、srt、vtt 或 ass 文件 |
| `subtitle_formats` | string[] | 这些同名字幕的格式，按 `subtitle` 接口的优先顺序排列：`vtt`、`srt`、`lrc`、`ass`。没有字幕时为空数组 |
| `video` | `VideoInfo` \| null | 视频轨道探测出的画面信息。不是视频或还没探测时为 `null` |
| `missing` | boolean | 文件已经找不到了。前端显示为灰色，不能播放 |
| `loudness` | `Loudness` \| null | 测出的响度，见“响度和波形”。还没测过、或者不是音频时为 `null`（视频不测） |

和另一个位置完全相同的重复文件不出现在列表里。

### `VideoInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `codec` | string | ffprobe 的视频编码名，比如 `h264`、`hevc` |
| `width` | number \| null | 像素宽度，ffprobe 读不出时为 `null` |
| `height` | number \| null | 像素高度 |

### `WorkDetail`

`WorkSummary` 的全部字段，再加上：

| 字段 | 类型 | 说明 |
|---|---|---|
| `vas` | `VaRef[]` | 声优，按名字排序 |
| `tags` | `TagRef[]` | 标签，按名字排序 |
| `total_duration_ms` | number \| null | 所有没有缺失的音频轨道时长之和。有任何一条还没探测、或者一条音频轨道都没有时为 `null` |
| `tracks` | `TrackItem[]` | 按自然排序排好的扁平列表 |
| `my_review` | `Review` \| null | 当前用户的评分和短评，没有时为 `null`，见“评分和短评” |
| `locations` | `WorkLocation[]` \| null | 作品在媒体库里的位置。媒体库的信息只给管理员看，所以普通用户拿到的是 `null` |

### `WorkLocation`

作品详情里的一个位置，管理员用它在网盘或文件夹里找到这部作品。没有缺失的位置排在前面，同一组里按媒体库 ID 和路径排序。缺失的作品也列出它所有缺失的位置。

| 字段 | 类型 | 说明 |
|---|---|---|
| `root_id` | string | 媒体库 ID |
| `root_name` | string | 媒体库的显示名 |
| `path` | string | 作品目录的显示路径，相对于媒体库，用 `/` 分隔，比如 `RJ01234567` 或 `社团/RJ01234567` |
| `missing` | boolean | 扫描时这个位置已经找不到了 |

### `PrepareRequest`

| 字段 | 类型 | 说明 |
|---|---|---|
| `can_play` | string[] | 浏览器 `canPlayType` 返回 `probably` 或 `maybe` 的 MIME 类型 |
| `force_fallback` | boolean | `true` 表示原格式播放失败过，一定要转码。可省略，默认 `false` |

### `PrepareResponse`

按 `state` 区分的两种形状：

| `state` | 其他字段 | HTTP |
|---|---|---|
| `ready` | `url`（string，相对地址，前端自己加 `?token=`），`mime_type`（string） | 200 |
| `processing` | `task_id`（number） | 202 |

### `TaskSnapshot`

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | |
| `kind` | string | `scan`、`probe`、`transcode`、`metadata`、`analysis` |
| `state` | string | `queued`、`running`、`done`、`failed`、`cancelled`、`interrupted` |
| `progress` | number \| null | 0 到 1，不知道时为 `null` |
| `message` | string \| null | 给人看的说明，失败时是原因 |
| `result` | `{url: string, mime_type: string}` \| null | 只有转码任务在 `done` 时有值。`analysis` 任务的结果用响度和波形接口取 |
| `queue_position` | number \| null | 排队中的扫描任务在扫描队列里排第几，1 表示正在运行的扫描结束后第一个开始。其他任务，包括排队中的其他种类的任务，都是 `null`。顺序可以用 `POST /api/tasks/{id}/move-to-front` 调整 |
| `created_at`、`updated_at` | number | |

### `CircleCount`、`VaCount` 和 `TagCount`

社团、声优、标签的列表条目，用于筛选。

| 类型 | 字段 |
|---|---|
| `CircleCount` | `id`（number）、`name`（string）、`work_count`（number） |
| `VaCount` | `id`（string）、`name`（string）、`work_count`（number） |
| `TagCount` | `id`（number）、`name`（string）、`work_count`（number） |

`work_count` 只数没有缺失的作品。

### `RootStatus`

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | 稳定标识，配置和数据库里的媒体库共用一个命名空间 |
| `display_name` | string | |
| `kind` | string | `local`、`openlist` 或 `webdav` |
| `status` | string | `ok`、`unavailable`、`unknown` |
| `last_scan_at` | number \| null | |
| `work_count` | number | 绑定在这个媒体库上的作品数 |
| `source` | string | `config`（写在 `config.toml` 里，接口只读）或 `db`（在界面里添加，存在数据库里）。见“媒体库管理接口” |
| `settings` | `RootSettings` | 这个媒体库的连接设置，不含密码 |
| `next_scan_at` | number \| null | 下一次自动扫描的时间（Unix 毫秒），见“自动扫描和目录监听”。没开自动扫描时是 `null`。时间已经过了、还在等前一次扫描结束时，仍然是那个过去的时间 |
| `watching` | boolean | 目录监听是否正在工作。没开 `watch`、OpenList 或 WebDAV 媒体库、监听失败时都是 `false` |

## 系统

### `GET /api/health`

不需要登录。进程活着就返回 200，`{"status": "ok"}`。

### `GET /api/ready`

不需要登录。就绪时返回 200，`{"status": "ready"}`。数据库还没迁移完或者内部服务没启动时返回 503，错误码 `not_ready`。

### `GET /api/version`

不需要登录。样本：`contract/m1/version.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `version` | string | 语义化版本号 |
| `git_commit` | string \| null | 构建时的提交，开发构建时可能为 `null` |
| `build_time` | number \| null | Nix 包使用提交时间的 Unix 毫秒值，不是实际构建时钟。直接 Cargo 构建时为 `null` |

## 认证

### `POST /api/auth/login`

不需要登录。请求样本：`contract/m1/login-request.json`，响应样本：`contract/m1/login-response.json`。

请求：

| 字段 | 类型 |
|---|---|
| `name` | string |
| `password` | string |

响应：

| 字段 | 类型 | 说明 |
|---|---|---|
| `token` | string | API token，放进 `Authorization: Bearer` |
| `expires_at` | number | 到期时间 |
| `user` | `User` | |

错误：

| HTTP | `code` | 什么时候 |
|---|---|---|
| 401 | `invalid_credentials` | 用户名或密码不对，或者这个账号还没设置密码。两种情况不区分 |
| 401 | `setup_required` | 库里还没有任何管理员。样本：`contract/m1/error-setup-required.json` |
| 429 | `too_many_attempts` | 同一个客户端地址在最近 1 到 2 分钟内已经失败 10 次。这时密码正确也返回 429。同一个用户名失败很多次，或者全局失败很多次，只会让请求变慢，不会返回 429。规则见[认证与用户](auth.md)的“密码错误的次数限制” |

### `GET /api/auth/me`

需要 API token。响应是 `User`，样本：`contract/m1/me.json`。token 无效时返回 401 `unauthorized`。

### `POST /api/auth/media-token`

需要 API token，没有请求体。响应样本：`contract/m1/media-token.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `token` | string | 媒体 token，拼成媒体 URL 的 `?token=` |
| `expires_at` | number | 到期时间 |

前端登录后换取媒体 token，媒体请求的权限边界见[认证与用户](auth.md)。

### `PUT /api/auth/password`

需要 API token。请求样本：`contract/m1/password-request.json`。成功返回 204，之后这个用户的所有 token 都失效，前端要跳回登录页。

| 字段 | 类型 | 说明 |
|---|---|---|
| `current_password` | string | |
| `new_password` | string | 至少 8 个字符 |

错误：当前密码不对时 401 `invalid_credentials`，新密码太短时 400 `invalid_request`。当前密码错误的次数和登录共用同一个限制，超过时返回 429 `too_many_attempts`。

### `POST /api/auth/logout-all`

需要 API token，没有请求体。让当前用户已经签发的所有 API token 和媒体 token 失效，包括这次请求用的这个，成功返回 204。之后这个用户在所有设备上都要重新登录，前端和改密码之后一样跳回登录页。媒体 token 不能调用它，返回 401 `unauthorized`。

改密码（`PUT /api/auth/password`）和服务器上的 `user set-password` 也都让这个用户的所有 token 失效，规则见[认证与用户](auth.md)。

## 作品与资料库

### `GET /api/works`

需要 API token。响应是 `WorkSummary` 的分页，样本：`contract/m1/works-page.json`。

样本里的三部作品分别展示了三种进度：有已对上的进度并且在重听、导入的旧进度还没对上、没有任何进度。

支持以下参数，可以任意组合：

| 参数 | 默认 | 说明 |
|---|---|---|
| `page` | `1` | 从 1 开始 |
| `page_size` | `40` | 最大 100，超过时按 100 处理 |
| `q` | 无 | 字面子串搜索，匹配编号、标题、社团名和声优名。ASCII 字母不分大小写，`%` 和 `_` 按普通字符处理。去掉首尾空白后为空时等于没有这个参数。超过 200 个字符时 400 `invalid_request` |
| `tag` | 无 | 标签 ID，只返回带这个标签的作品 |
| `circle` | 无 | 社团 ID |
| `va` | 无 | 声优 ID（字符串） |
| `root` | 无 | 媒体库 ID，只返回在这个媒体库里有没缺失的位置的作品 |
| `sort` | `added` | `added`（入库时间）、`release`（发售日）、`rating`（DLsite 平均分）、`random` |
| `order` | `desc` | `desc` 或 `asc`。`random` 不看这个参数 |
| `seed` | 无 | `sort=random` 时必须给，非负整数。同一个 `seed` 在同一份数据下给出同一个顺序，所以翻页不会重复或遗漏 |
| `include_missing` | `false` | `true` 时缺失的作品也返回，`missing` 为 `true` |

每个筛选参数只接受一个值。没有发售日或平均分的作品，在 `release` 和 `rating` 排序里不管升序降序都排在最后。所有排序最后都按作品 ID 决胜。

错误：参数值不对（比如 `tag` 不是正整数、`sort` 不认识、`sort=random` 没给 `seed`）时 400 `invalid_request`。按不存在的标签、社团、声优或媒体库筛选不是错误，返回空列表。

### `GET /api/works/{id}`

需要 API token。响应是 `WorkDetail`，样本：`contract/m2/work-detail.json`。作品不存在时 404 `not_found`，所有位置都缺失时仍然返回 200，`missing` 为 `true`。

### `GET /api/works/{id}/cover`

需要 API token（`Authorization` 头）或媒体 token。媒体 token 可以放在 `?token=` 里，也可以放在 cookie 里：HTTPS 下叫 `__Secure-kikoeru-media`，纯 HTTP 下叫 `kikoeru-media`，只有这个接口读它们。分成两个名字是因为 HTTPS 写下的 `Secure` cookie 不能被同一主机的 HTTP 页面覆盖。两个都在时逐个验证，用第一个有效的。有 `?token=` 时只用 `?token=`。前端用 cookie，封面地址里不带 token，所以换了媒体 token 之后地址不变，浏览器已经缓存的封面继续有效。返回图片字节。参数 `size=thumb` 返回宽 480 像素的缩略图，不带参数返回原图。参数 `v` 是 `cover_version`，只用来让缓存失效，服务端不检查它的值。`has_cover` 为 `false` 时返回一张 SVG 占位图，状态码 200，`Cache-Control: private, no-cache`，这样封面出现后马上能看到。封面图片的回答带 `Cache-Control: private, no-cache` 和 `ETag`（由作品、`cover_version` 和大小决定），请求带上匹配的 `If-None-Match` 时返回 304，没有响应体。每次加载都要回服务器确认，是因为封面地址不带 token：浏览器缓存里的封面若不经确认就能显示，退出登录后同一个浏览器的人照样能打开。确认请求也要带有效的 token，没有就是 401。

### `GET /api/circles`、`GET /api/vas` 和 `GET /api/tags`

需要 API token。不分页，按名字排序，同名按 ID 排序。只列至少有一部没缺失的作品的条目。响应分别是 `CircleCount[]`、`VaCount[]`、`TagCount[]`，样本：`contract/m3/circles.json`、`contract/m3/vas.json`、`contract/m3/tags.json`。

## 播放与媒体

### `POST /api/tracks/{id}/prepare`

需要 API token。请求样本：`contract/m2/prepare-request.json`。响应是 `PrepareResponse`，样本：`contract/m2/prepare-ready.json`（200）和 `contract/m2/prepare-processing.json`（202）。

错误：轨道不存在 404 `not_found`。轨道缺失 410 `missing`。队列已满 429 `queue_full`。磁盘不足 507 `insufficient_storage`。媒体库连不上 503 `storage_unavailable`。轨道的 `kind` 不是 `audio` 或 `video` 时返回 400 `invalid_request`。

### 媒体字节

下面这些接口返回文件内容，不是 JSON。都接受 API token 或媒体 token（`?token=`），都只允许 GET 和 HEAD，Range 规则见下文。出错时仍然返回 JSON 错误体。

| 路径 | 内容 | 说明 |
|---|---|---|
| `/api/tracks/{id}/stream` | 原始文件 | `Content-Type` 按扩展名和 probe 结果 |
| `/api/tracks/{id}/playable` | 转码后的文件 | 没准备好时 409 `media_not_ready`，样本：`contract/m2/error-media-not-ready.json` |
| `/api/tracks/{id}/download` | 原始文件 | 加 `Content-Disposition: attachment`，文件名用原文件名 |
| `/api/tracks/{id}/subtitle` | WebVTT 文本 | `Content-Type: text/vtt; charset=utf-8`。没有字幕时 404 `not_found`。不带 `format=ass` 时，ass 转成 WebVTT，只保留文字 |
| `/api/works/{id}/download` | zip 流 | 参数 `group` 是目录分组，不带时打包整部作品。文件名 `{code}.zip` 或 `{code}-{分组最后一段}.zip`。所有用户合计最多同时进行 2 个打包下载，名额已满时立即返回 429 `queue_full`，不进入等待队列，这个限额独立于转码队列 |

`stream` 和 `playable` 支持普通 Range、后缀 Range、越界时的 416、`If-Range`、`ETag` 和 `Last-Modified`。多区间 Range 按整个文件返回 200。远端转发遵守[存储后端](storage.md)的规则。zip 流不支持 Range。HEAD 不触发探测或转码。

## 用户进度与收藏

### `PUT /api/me/progress/{work_id}`

需要 API token。请求是 `ProgressRequest`。有序请求样本为 `contract/m2/progress-request.json`，显式 `null` 的样本为 `contract/m2/progress-request-null.json`。

服务端先执行鉴权和请求合法性检查，再判断报告是否过期或重复。无效的排序对象、会话标识或序号返回 400 `invalid_request`，包括排序对象缺少成员、成员类型错误、会话标识格式不符和序号不在允许范围内。现有轨道归属、位置和分组检查继续适用：`track_id` 不属于这部作品、`position_ms` 不是允许范围内的整数，或 `group_path` 超过 4096 字节时返回 400 `invalid_request`。过期或重复的序号不能使无效请求绕过这些检查。

对携带排序对象的合法报告，服务端按用户、作品和会话保存已成功接受的最大序号。首次报告或序号更大的报告更新进度，并在同一事务中推进序号，任一写入失败时两者一起回滚。序号小于或等于已保存序号的报告返回 204，不修改轨道、位置、分组、完成状态、重听状态或更新时间。更大的序号可以报告更小的位置，因此用户往回拖动仍然有效。

服务端先排除各会话中已过期或重复的报告。不同会话中其余有效报告按实际成功写入顺序覆盖，不比较不同会话的序号，也不规定新页面或某台设备优先。其他会话的写入不会替换已有会话的序号记录。例如，A2 表示会话 A 的序号 2，`A2 → B1 → A1` 中的 A1 和 `A2 → B1 → A2` 中最后的 A2 都不会覆盖 B1 写入的进度。

排序字段为 `null` 或缺失时，报告沿用原有覆盖行为，但不删除或降低任何会话的序号记录。旧格式报告本身不享有乱序保护，仍可能覆盖新版客户端写入的进度。

清除进度和标记完成不会清除会话序号记录，已知旧包或重复包不能重建已清除的进度或改变完成状态。更大的新序号仍可以记录新的播放进度。当前只在用户或作品被删除时级联清除对应序号记录，不按时间过期。此规则不为 PUT、清除进度和标记完成建立跨操作或跨会话的统一顺序，尚未被同会话更大序号覆盖的报告仍可能在清除进度或标记完成之后首次写入。

204 表示请求已经处理，也可能表示报告已经过期或重复，不承诺当前进度等于本次请求的位置。页面退出时的请求可能没有送达，服务端只保证已送达报告的上述处理规则。

### `POST /api/me/progress/{work_id}/finish`

需要 API token。请求样本：`contract/m2/finish-request.json`，字段 `track_id` 和 `group_path`。后端确认这条轨道是这个分组按自然排序的最后一条音频或视频，然后设置 `finished_at`、清掉 `replaying`，返回 204。不是最后一条时 400 `invalid_request`。作品详情页的“标记为听过”也走这个接口，传当前轨道和分组，这时不检查是不是最后一条，请求体加 `"manual": true`。

### `DELETE /api/me/progress/{work_id}`

需要 API token。清除进度，返回 204。没有进度时也返回 204。

### `GET /api/me/favorites`

需要 API token。当前用户的收藏，按加入收藏的时间倒序，同时间按作品 ID 倒序。参数 `page`、`page_size` 和作品列表一样。响应是 `WorkSummary` 的分页，样本：`contract/m3/favorites-page.json`。缺失的作品也在列表里，`missing` 为 `true`，因为收藏是用户自己的数据。

### `PUT /api/me/favorites/{work_id}` 和 `DELETE /api/me/favorites/{work_id}`

需要 API token。加入或取消收藏，成功返回 204，重复调用没有副作用。作品不存在时 404 `not_found`。

### `GET /api/me/history`

需要 API token。当前用户有播放进度的作品，按进度的更新时间倒序，同时间按作品 ID 倒序。参数和响应同收藏列表，样本：`contract/m3/history-page.json`。缺失的作品也在列表里。清除进度之后作品就不在这里了。

## 扫描与元数据管理

### `POST /api/admin/scan`

需要管理员的 API token。请求体 `{"root": "local"}`，`root` 缺失或为 `null` 时，为所有已成功加载存储后端的媒体库提交扫描，跳过后端加载失败的媒体库。显式指定后端加载失败的媒体库时返回 503 `storage_unavailable`。可选的 `"full": true` 让 OpenList 和 WebDAV 媒体库完整遍历每部作品，不按目录摘要跳过没变的作品（见[存储后端](storage.md)的“列目录优化”）。对本地媒体库没有影响。响应 202 `{"task_ids": [number]}`，样本：`contract/m2/scan-response.json`。某个媒体库已经在扫描时，返回正在运行的那个任务的 ID，即使两次请求的 `full` 不同。`root` 不存在时 404 `not_found`。

`full` 不强制刷新 OpenList 缓存，只影响本服务是否跳过目录摘要未变的作品。

### `PUT /api/works/{id}/metadata`

需要管理员的 API token。整体替换一部作品的元数据，之后 `metadata_src = 'manual'`，普通的重新抓取不再覆盖它。请求样本：`contract/m4/metadata-request.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `title` | string | 不能为空 |
| `release_date` | string \| null | `YYYY-MM-DD` |
| `nsfw` | boolean | |
| `circle_name` | string \| null | 按名字找社团，没有就新建 |
| `va_names` | string[] | 按名字找声优，没有就新建 |
| `tag_names` | string[] | 先按别名找规范标签，没有就新建一个规范标签并把这个名字记成别名 |

成功返回 200 和更新后的 `WorkDetail`。作品不存在 404 `not_found`，字段不合法 400 `invalid_request`。

### `POST /api/works/{id}/refresh`

需要管理员的 API token。重新抓取这部作品的元数据和封面，返回 202 和 `{"task_id": number}`，样本：`contract/m4/refresh-response.json`。同一部作品已经在抓取时返回那个任务。参数 `force=true` 时也覆盖 `manual` 的元数据和 `imported`、`manual` 的封面，不带时这些都保持不变。作品不存在 404 `not_found`。

### `GET /api/admin/missing`

需要管理员的 API token。缺失的作品和缺失的位置，样本：`contract/m4/missing.json`。作品列表里是所有位置都缺失的作品，位置列表里是作品本身没缺失、但这个位置缺失了的位置。两个列表都按缺失时间倒序。

| 类型 | 字段 |
|---|---|
| `MissingWork` | `id`、`code`、`title`、`missing_at` |
| `MissingLocation` | `id`、`work_id`、`code`、`root_id`、`path`（显示路径）、`missing_at` |

### `POST /api/admin/missing/cleanup`

需要管理员的 API token。物理删除选中的缺失作品和缺失位置，请求样本：`contract/m4/cleanup-request.json`，响应样本：`contract/m4/cleanup-response.json`。只删除现在仍然缺失的行，期间重新出现的会被跳过，不计入删除数。删除作品会一并删除它的轨道、收藏、播放进度和导入辅助记录，这是不能撤销的操作，前端要先让用户确认。删除位置会删除这个位置的轨道，指向这些轨道的播放进度改成没有轨道。有扫描正在运行时返回 409 `scan_running`，因为扫描跨事务持有作品和位置的快照，中途删掉会让扫描出错。排队中的扫描会等清理结束再开始。一次最多 1000 个 ID（作品和位置合计），超过时 400 `invalid_request`。

### `POST /api/admin/tags/merge`

需要管理员的 API token。把标签 `from` 合并进标签 `into`，请求样本：`contract/m4/tag-merge-request.json`。`from` 的作品关联和别名都转到 `into`，然后删除 `from`。成功返回 204。两个 ID 相同或有一个不存在时 400 `invalid_request`。

## 任务

### `GET /api/tasks/{id}`

需要 API token。响应是 `TaskSnapshot`，样本：`contract/m2/task-running.json` 和 `contract/m2/task-done.json`。普通用户只能查自己通过 `prepare`、响度或波形接口触发的转码和 `analysis` 任务，查别的返回 404 `not_found`。

### `GET /api/tasks`

需要管理员的 API token。所有任务，按创建时间倒序，同时间按 ID 倒序。参数 `page`、`page_size` 和作品列表一样，另外可以用 `state` 只看一种状态，用 `exclude_kind` 去掉一种任务，取值是 `TaskKind` 里的一个，比如 `exclude_kind=analysis` 不列响度分析。两个参数可以一起用，`total` 也按同样的条件计数。取值不认识时返回 400 `invalid_request`。响应是 `TaskSnapshot` 的分页，样本：`contract/m4/tasks-page.json`。

### `POST /api/tasks/{id}/cancel`

需要管理员的 API token。请求取消，返回 204。任务会在安全的时候停下来，最终状态是 `cancelled`。任务不存在 404 `not_found`，已经结束 400 `invalid_request`。

### `POST /api/tasks/{id}/move-to-front`

需要管理员的 API token。把一个排队中的扫描任务移到扫描队列的最前面，返回 204。扫描同时只跑一个，排队的扫描默认按提交的先后开始，这个接口让管理员先扫描某一个媒体库。正在运行的扫描不受影响，要它让开只能取消它。排在它前面的扫描各往后挪一位，其余的顺序不变。新的顺序写进数据库，服务重启后排队中的扫描仍按这个顺序开始。已经排在最前面时什么都不变，同样返回 204。任务不存在 404 `not_found`。不是扫描任务，或者已经开始、已经结束，400 `invalid_request`。排队中的扫描任务的样本：`contract/m4/task-queued-scan.json`。

### `GET /api/tasks/{id}/events`

需要管理员的 API token，放在 `Authorization` 头里，所以前端用 `fetch` 读这个流，不用 `EventSource`。响应是 `text/event-stream`：

- 连上之后先发一个 `snapshot` 事件，`data` 是当前的 `TaskSnapshot`。
- 之后状态、进度、说明或排队位置每次变化都再发一个 `snapshot` 事件，进度最多每秒一次。
- 任务结束（`done`、`failed`、`cancelled`、`interrupted`）时发最后一个 `snapshot`，然后服务端关闭连接。
- 没有变化时每 15 秒发一个注释行（`: ping`），让代理不断开连接。

断开连接只结束订阅，不会取消任务。前端断线重连时重新请求这个地址，第一条事件就是最新的快照。任务不存在 404 `not_found`。所有用户加起来最多同时开 16 个这样的连接，再多返回 429 `queue_full`。

## 视频与字幕

视频播放和 ass 字幕。`TrackItem` 包含 `subtitle_formats` 和 `video` 字段，见上文。样本在 `contract/m6/`。

### 视频的 `prepare`

请求使用 `PrepareRequest`，仍须发送顶层 `can_play`，不能只发送 `force_fallback`。后端按[媒体处理与播放](media.md)的规则决定：

| 情况 | 响应 |
|---|---|
| 浏览器能直接播放原文件（mp4 里的 H.264 + AAC 这类，按 `can_play` 判断） | 200 `ready`，`url` 是 `/api/tracks/{id}/stream`，`mime_type` 比如 `video/mp4` |
| 其他容器里的 H.264 + AAC | 转封装成 HLS（不重新编码） |
| 其他编码，或者 `force_fallback` 为 `true` | 转码成 H.264 + AAC 的 HLS，画面不超过 1080p，视频码率不超过 8 Mbps |
| 要新转码，但 `config.toml` 里 `media.video_transcode = false` | 403 `forbidden`，`message` 说服务器关闭了视频转码。缓存里已有的转码照样返回 |

HLS 还没生成时返回 202 `processing`，任务的 `kind` 仍是 `transcode`，完成后 `result` 是 `{"url": "/api/tracks/{id}/hls/index.m3u8", "mime_type": "application/vnd.apple.mpegurl"}`，样本：`contract/m6/task-done-hls.json`。HLS 已经在缓存里时直接返回 200 `ready`，样本：`contract/m6/prepare-ready-hls.json`。

### `GET /api/tracks/{id}/hls/index.m3u8`

HLS 播放列表，接受 API token 或媒体 token（`?token=`），只允许 GET 和 HEAD。`Content-Type: application/vnd.apple.mpegurl`，`Cache-Control: private, no-store`。分片是 6 秒左右的 MPEG-TS，播放列表是点播类型（`#EXT-X-PLAYLIST-TYPE:VOD`，带 `#EXT-X-ENDLIST`），样本：`contract/m6/playlist.m3u8`。还没生成时 409 `media_not_ready`。

播放列表里每个分片的地址都是相对地址，形如 `<组>/seg-00000.ts?token=<媒体 token>`。`<组>` 是这组 HLS 的标识：缓存键的摘要（64 位十六进制）加上 `-` 和这一组的生成时间（纳秒），它确定了来自哪个文件、哪个版本、做了哪种处理、是哪一次生成的。没有可靠修改时间的网盘文件过期后重新生成时，会得到新的标识。播放列表指向哪一组，它的分片就一直从那一组取：播放中途另一种处理（转封装或转码）生成好了，或者源文件换了新版本，都不会换成分段边界不同的另一组。转封装和转码两组都在时，新请求的播放列表用转码的那组。token 的来源：

- 请求用 `?token=` 带了媒体 token 时，就用这个 token。
- 请求用 `Authorization` 头带 API token 时，后端为这个用户签发一个新的媒体 token 写进分片地址，不会把 API token 写进 URL。

为什么这样做：播放器（Android Chrome 的原生 HLS 和桌面上的 hls.js 一样）只按播放列表里的地址去取分片，不会把播放列表 URL 上的查询参数带过去，所以只在播放列表上带 token 不够。另一种做法是给每个分片单独签名的路径，但那需要再引入一套签名和密钥。改写播放列表复用现有的媒体 token：受众限定为媒体、6 小时过期、改密码或删用户时随 `token_version` 一起失效，原生 HLS 和 hls.js 都不需要额外配置。代价是播放列表不能缓存（所以是 `no-store`），以及 token 出现在分片地址里，访问日志照样去掉 `token` 参数。媒体 token 在播放中途过期时，分片请求得到 401，播放器按[认证与用户](auth.md)的规则换一个媒体 token，重新请求播放列表，再从原来的位置继续。

### `GET /api/tracks/{id}/hls/{set}/{segment}`

一个分片。`set` 是播放列表里的组标识，`segment` 形如 `seg-00000.ts`，其他名字返回 404 `not_found`。认证和播放列表一样。`Content-Type: video/mp2t`，支持 Range，规则见 [API 调用流程](api.md#先准备再播放)。这一组不存在或者已经被淘汰时返回 409 `media_not_ready`，播放器重新调用 `prepare`。

### `GET /api/tracks/{id}/subtitle?format=ass`

`subtitle_formats` 里有 `ass` 时，原样返回这条轨道的 ass 字幕，给前端的 JASSUB 渲染。文本按 `subtitle` 接口同样的规则解码，统一转成 UTF-8 返回，`Content-Type: text/plain; charset=utf-8`。没有 ass 字幕时 404 `not_found`。不带 `format` 时行为不变，返回优先级最高的字幕转出的 WebVTT。媒体 token 可以访问。

### `GET /api/tracks/{id}/fonts`

需要 API token。这条轨道所属作品里的字体文件（扩展名 `ttf`、`otf`、`ttc`、`woff`、`woff2`，所有位置和子目录，完全相同的重复文件只列一份，缺失的不列），给 JASSUB 加载 ass 里用到的字体。响应是 `TrackFont[]`，按路径的自然排序，最多 64 个，样本：`contract/m6/fonts.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | number | 字体文件的轨道 ID |
| `name` | string | 文件名 |
| `url` | string | `/api/tracks/{id}/stream`，前端自己加 `?token=` |

轨道不存在 404 `not_found`。没有字体时返回空数组。

## 响度和波形

播放器按每条音频轨道的响度调整音量（Web Audio 的 GainNode），桌面宽屏布局在进度条上画波形。视频轨道不测，也没有波形。两样都由一次 `analysis` 任务算出，算法和缓存见[媒体处理与播放](media.md)的“响度和波形”。样本在 `contract/loudness/`。

### `Loudness`

| 字段 | 类型 | 说明 |
|---|---|---|
| `integrated_lufs` | number \| null | 整条轨道的积分响度（EBU R128，LUFS），保留一位小数。几乎全是静音、测不出时为 `null` |
| `true_peak_dbtp` | number \| null | 真峰值（dBTP），保留一位小数。全是静音时为 `null` |

前端的增益是 `target_lufs - integrated_lufs` dB，再限制到不让 `true_peak_dbtp` 加上增益超过 -1 dBTP。`integrated_lufs` 为 `null` 时不调整。

### `GET /api/tracks/{id}/loudness`

需要 API token。已经测过时返回 200 和 `Loudness`，样本 `contract/loudness/loudness.json`。还没测过时排一个 `analysis` 任务，返回 202 和 `{"state": "processing", "task_id": number}`，样本 `contract/loudness/loudness-processing.json`，任务 `done` 之后再请求一次。远端媒体库的文件正在转码时，`task_id` 是那个转码任务：转码完成后服务用转码结果测响度，文件只下载一次。客户端的做法不变，等任务结束再请求一次，这时一般会拿到新排的 `analysis` 任务。

错误：轨道不存在 404 `not_found`。不是音频（包括视频）404 `not_applicable`。轨道缺失 410 `missing`。队列已满 429 `queue_full`。磁盘不足 507 `insufficient_storage`。媒体库连不上 503 `storage_unavailable`。

### `GET /api/tracks/{id}/waveform`

需要 API token。算好时返回 200，`Content-Type: application/octet-stream`，正文正好 1000 个字节，第 i 个字节是轨道第 i 个千分之一时段里的最大振幅，0 到 255 线性对应 0 到满幅。带 `ETag` 和 `Cache-Control: private, max-age=604800`，`If-None-Match` 对得上时返回 304。还没算好时和 `loudness` 一样排 `analysis` 任务并返回 202，错误也相同。波形放在缓存里，被淘汰之后再请求会重新排任务，响度存在数据库里不会丢。

### `analysis` 任务

- 同一条轨道同时只有一个 `analysis` 任务（`dedupe_key` 是 `analysis:{轨道 ID}`），走轻任务通道（`media.light_jobs`），排在探测之后。
- 对音频轨道调用 `prepare` 时，如果它还没有响度，服务顺便排一个 `analysis` 任务，不等它完成，`prepare` 的回答和以前一样。远端媒体库的文件要转码时，`prepare` 不排，转码任务完成后再排，分析读转码结果。扫描从不排 `analysis`，但读取文件夹封面时仍会下载候选图片。它们的轨道在第一次播放（`prepare`）或者请求响度、波形接口时才测。
- 普通用户能查自己触发的 `analysis` 任务，规则和转码任务一样。

## 个人设置

### `UserSettings`

| 字段 | 类型 | 说明 |
|---|---|---|
| `normalize_loudness` | boolean | 是否按响度调整音量，默认 `false` |
| `target_lufs` | number | 目标响度，默认 `-16`，范围 -30 到 -10 |
| `accent` | `Accent` \| null | 界面的强调色，`null` 表示默认的蓝色，见下文 |
| `seek_back_seconds` | number | 后退按钮一次退多少秒，1 到 99 的整数，默认 `15` |
| `seek_forward_seconds` | number | 前进按钮一次进多少秒，1 到 99 的整数，默认 `30` |

### `Accent`

字符串，只能是下面八个名字之一，对应 iOS 的同名系统色：`"blue"`、`"purple"`、`"pink"`、`"red"`、`"orange"`、`"green"`、`"teal"`、`"indigo"`。不接受任意的颜色值，因为每个名字在浅色和深色主题下各有一组前端定好的色值，保证两种主题下文字和按钮都看得清，具体色值见[前端](frontend.md)。`"blue"` 和 `null` 显示效果相同，保存的值照原样返回。强调色跟着账号走，在哪台设备上登录都一样。

### `GET /api/me/settings` 和 `PUT /api/me/settings`

需要 API token。`GET` 返回当前用户的 `UserSettings`，没有保存过时返回默认值，样本 `contract/loudness/user-settings.json`。`PUT` 的请求体是完整的 `UserSettings`，整体替换，成功返回 200 和保存后的值。`target_lufs` 超出范围或不是数字时 400 `invalid_request`。`accent` 不是上面的名字之一时同样 400 `invalid_request`。请求体里没有 `accent` 时按 `null` 处理，这样加这个字段之前的客户端仍然能保存响度设置，只是强调色回到默认。`seek_back_seconds` 或 `seek_forward_seconds` 不是 1 到 99 的整数时 400 `invalid_request`，请求体里没有它们时按默认的 15 和 30 秒处理，理由和 `accent` 相同。两个秒数跟着账号走，播放页、迷你播放条和视频控制栏的按钮都用它们，锁屏和耳机上的快退快进在系统没有指定秒数时也用它们。上限是 99，因为按钮图标里只放得下两位数。

## 评分和短评

每个用户可以给作品打 1 到 5 星，再写一段短评，只有自己看得到。作品卡片在 `my_rating` 不为空时显示自己的星数，详情页显示并可以修改评分和短评，“我的评价”页列出评过的作品。DLsite 的平均分（`rate_average`）和这里无关，照旧显示。存储和导入见[数据模型](data-model.md)的 `reviews` 和[迁移](migration.md)。样本在 `contract/reviews/`。

### `Review`

| 字段 | 类型 | 说明 |
|---|---|---|
| `rating` | number \| null | 1 到 5 的整数，只写了短评时为 `null` |
| `text` | string \| null | 短评，只打了分时为 `null` |
| `updated_at` | number \| null | 最后修改时间。从旧库导入、还没在新版改过的记录，旧库也没有时间时为 `null` |

`rating` 和 `text` 不会同时为 `null`，两样都没有时整个 `Review` 就是 `null`。

### `ReviewInput`

| 字段 | 类型 | 说明 |
|---|---|---|
| `rating` | number \| null | 1 到 5 的整数，`null` 表示不打分 |
| `text` | string \| null | 短评，去掉首尾空白之后最多 2000 个字符（按 Unicode 码位算）。空字符串和 `null` 一样，表示没有短评 |

### `PUT /api/me/reviews/{work_id}`

需要 API token。请求体是 `ReviewInput`（样本 `contract/reviews/review-request.json`），整体替换这部作品上自己的评分和短评，成功返回 200 和保存后的 `Review`（样本 `contract/reviews/review.json`）。重复提交同样的内容也会更新 `updated_at`。

错误：作品不存在时 404 `not_found`。`rating` 不是 1 到 5 的整数、短评超长、或者 `rating` 和 `text` 都为空时 400 `invalid_request`，要清除用 `DELETE`。缺失的作品也可以评，和收藏一样。

### `DELETE /api/me/reviews/{work_id}`

需要 API token。清除这部作品上自己的评分和短评，成功返回 204，没有评过也返回 204。作品不存在时 404 `not_found`。

### `GET /api/me/reviews`

需要 API token。当前用户评过的作品，按 `updated_at` 倒序，`updated_at` 为 `null` 的排在最后，同时间按作品 ID 倒序。参数 `page`、`page_size` 和作品列表一样。响应是 `WorkSummary` 的分页，样本 `contract/reviews/reviews-page.json`。缺失的作品也在列表里。短评不在这个列表里，要看全文进详情页。

## 媒体库管理接口

在界面里添加、修改、删除媒体库。桌面版只用这种方式管理媒体库，服务器版也可以继续只写 `config.toml`。服务从不修改配置文件：界面添加的媒体库存在数据库里，`source` 是 `db`。`config.toml` 里的媒体库 `source` 是 `config`，接口只能看，不能改，要改就改配置文件再重启。两种来源的 `id` 共用一个命名空间，不能重复。启动时如果配置文件里新加了一个和数据库里重名的媒体库，以配置文件为准，数据库里那一条不生效，日志里给出警告。

增删改立刻生效，不需要重启：正在扫描或播放这个媒体库的请求要么用旧设置跑完，要么以 `storage_unavailable` 结束，不会读到一半换成另一个位置的文件。

### `RootSettings`

按 `type` 区分的三种形状，是 `RootStatus.settings` 的类型，样本见 `contract/m7/roots.json` 和 `contract/webdav/roots.json`：

| `type` | 其他字段 |
|---|---|
| `local` | `path`（string，绝对路径），`watch`（boolean），`auto_scan_interval`（number \| null） |
| `openlist` | `url`（string），`base_path`（string），`username`（string），`password_set`（boolean，是否已经保存了密码），`list_concurrency`（number \| null），`link_rate`（number \| null），`auto_scan_interval`（number \| null） |
| `webdav` | `url`（string），`base_path`（string），`username`（string），`password_set`（boolean），`list_concurrency`（number \| null），`auto_scan_interval`（number \| null）。没有 `link_rate`，因为 WebDAV 没有取直链这一步 |

`watch` 和 `auto_scan_interval` 见“自动扫描和目录监听”。`watch` 是生效的值，没写时按运行模式取默认值后的结果。

密码只写不读：任何响应都不包含密码，只用 `password_set` 说明有没有。

### `RootInput`

`POST` 和 `PUT` 的请求体，样本：`contract/m7/root-create-request.json` 和 `contract/webdav/root-create-request.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | 只在 `POST` 里出现。1 到 32 个字符，只用小写字母、数字、`-`、`_`，以字母或数字开头。创建后不能改 |
| `display_name` | string | 不能为空，最多 100 个字符 |
| `settings` | object | 按 `type` 区分：`{"type": "local", "path": ...}` 、`{"type": "openlist", "url", "base_path", "username", "password", "list_concurrency", "link_rate"}` 或 `{"type": "webdav", "url", "base_path", "username", "password", "list_concurrency"}`。`openlist` 和 `webdav` 的 `password` 在 `POST` 时必填，不能为空，也不能包含换行。`PUT` 时不写表示沿用已保存的密码，但只限 `url` 的协议、主机和端口都没变，换了服务器必须重新填写，免得保存的密码被发到别的地址。`list_concurrency` 和 `link_rate` 可以不写，用默认值。三种类型都可以带 `auto_scan_interval`，不写或 `null` 表示不自动扫描。`local` 还可以带 `watch`，不写或 `null` 表示按运行模式取默认值。`PUT` 整体替换设置，所以 `PUT` 里不写这两个字段也是回到默认，不是沿用原来的值 |
| `skip_check` | boolean | 可选，默认 `false`。为 `true` 时 OpenList 或 WebDAV 连不上也照样保存 |

保存前的检查：本地媒体库的路径必须是已经存在的目录。OpenList 媒体库会用给的账号登录，并列一次 `base_path`。WebDAV 媒体库用给的账号对 `base_path` 发一次 `Depth: 0` 的 `PROPFIND`，要求它是一个目录。连不上时的错误和 OpenList 相同（503 `storage_unavailable` 或 502 `upstream_error`）。

### 自动扫描和目录监听

两个按媒体库设置的开关，让新放进去的文件不用手动点扫描也能出现。两种来源的媒体库都能设置：`config.toml` 写在 `[[roots]]` 下面，示例见[使用指南的“用文件声明媒体库”](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/libraries.md#用文件声明媒体库)。界面添加的写在 `RootInput.settings` 里。

- `auto_scan_interval`：每隔多久自动扫描一次，接口里的单位是秒，`config.toml` 里写成 `"6h"`、`"30m"` 这样的时长。不写或 `null` 表示不自动扫描，这是默认值。最短间隔 OpenList 和 WebDAV 是 30 分钟（`1800`），免得频繁列远端目录。本地是 5 分钟（`300`）。最长 30 天（`2592000`）。超出范围时，`config.toml` 让服务启动失败，接口返回 400 `invalid_request`。
- `watch`：只对本地媒体库有效，监听目录里的文件变化。桌面版的本机模式默认 `true`，服务器版默认 `false`。OpenList 和 WebDAV 媒体库写了 `watch = true` 时同样启动失败或返回 400。

自动扫描排的是普通的扫描任务，和调用 `POST /api/admin/scan`（不带 `full`）一样，也出现在任务列表里。这个媒体库已经有扫描在排队或运行时，这一次跳过，不另排一个。下一次的时间从上一次扫描结束时算起，不管那次是手动、自动还是监听触发的，也不管成功还是失败，所以重启服务不会让间隔重新计时。从没扫描过的媒体库从服务启动、添加媒体库或开启自动扫描时算起。重启时已经过了时间的，启动后马上排队。

监听到变化后，等这个媒体库连续 10 秒没有新变化，再排一次整个媒体库的扫描。本地扫描只读本机磁盘，没有变化的作品很快就过去了。变化发生时已经有扫描在运行的，那次扫描结束后再排一次，免得漏掉它已经走过的目录。监听失败（比如系统允许的监听数量用完，或者网络共享根本不发通知）只写日志，不影响服务，`watching` 变成 `false`。这时如果设了 `auto_scan_interval`，照常按间隔扫描，所以网络共享上的本地媒体库建议同时设一个间隔。

OpenList 和 WebDAV 的密码存在数据目录的 `secrets/roots/`，权限 `0600`，和签名密钥一样随[备份](config.md)一起导出。数据库在同一次提交中切换地址和密码文件的版本引用，旧目录的未版本化文件仍可读取，发布和回收顺序见[配置与持久状态](config.md#媒体库密码的发布与回收)。停服开始后不再接收新的媒体库修改，新进入处理函数的增删改请求返回 503 `not_ready`。

### `GET /api/admin/roots`

需要管理员的 API token，普通用户返回 403 `forbidden`。响应是 `RootStatus[]`，包含 `source` 和 `settings`，样本：`contract/m7/roots.json` 和 `contract/webdav/roots.json`。先列配置里的媒体库（配置文件的顺序），再列数据库里的（创建顺序）。

### `POST /api/admin/roots`

需要管理员的 API token。请求体是 `RootInput`，成功返回 201 和新媒体库的 `RootStatus`。新媒体库还没扫描过，状态是 `unknown`，要再调 `POST /api/admin/scan` 扫描。

错误：

- 400 `invalid_request`：`id`、`display_name` 或设置不合法，本地路径不存在或不是目录，OpenList 或 WebDAV 缺少密码。
- 409 `root_exists`：已经有这个 `id`，不管是配置里的还是数据库里的。
- 503 `storage_unavailable` 或 502 `upstream_error`：OpenList 或 WebDAV 连不上、登录失败或者无法检查 `base_path`，`message` 写出原因。带上 `"skip_check": true` 可以照样保存。

### `PUT /api/admin/roots/{id}`

需要管理员的 API token。请求体是没有 `id` 的 `RootInput`，整体替换显示名和设置，成功返回 200 和更新后的 `RootStatus`。换了位置或网盘时，已经扫描过的作品不会自动迁移，下一次扫描按新位置重新发现，旧位置里找不到的文件按缺失规则标记。

错误：媒体库不存在 404 `not_found`。配置里的媒体库 409 `root_read_only`。这个媒体库正在扫描或排队扫描时改路径、服务器、`base_path` 或用户名 409 `scan_running`，只改显示名和限速不受影响。其他和 `POST` 相同。

### `DELETE /api/admin/roots/{id}`

需要管理员的 API token。删除一个数据库里的媒体库，成功返回 204。这个媒体库的所有位置和轨道马上标记为缺失，作品的其他位置都缺失时作品也标记为缺失，收藏和播放进度都保留。它从 `GET /api/admin/roots` 里消失，服务拥有的版本化密码文件在确认没有已提交行或配置媒体库引用后删除，无法证明已被版本化引用替代的旧格式文件保留，回收失败或中途停机时在后续修改或启动重试。真正删除这些缺失的记录，走已有的 `POST /api/admin/missing/cleanup`。以后用同样的 `id` 重新添加并扫描，找到的文件按原来的轨道恢复。

错误：媒体库不存在 404 `not_found`。配置里的媒体库 409 `root_read_only`。这个媒体库正在扫描或排队扫描时 409 `scan_running`，先取消扫描任务。

## 服务器设置

整个服务共用的设置，只有管理员能看和改。它们存在数据库里，和界面添加的媒体库一样随备份导出，改了马上生效，不用重启。目前只有元数据代理一项。样本在 `contract/settings/`。

### `ServerSettings`

样本 `contract/settings/server-settings.json`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `metadata_proxy` | string \| null | 抓取元数据和封面时用的代理，比如 `http://127.0.0.1:7890` 或 `socks5://127.0.0.1:1080`。`null` 表示没有设置 |

`metadata_proxy` 管的是 DLsite、asmr.one 和 hvdb 的请求，包括从它们下载封面。媒体库（OpenList、WebDAV）的请求不走它，因为媒体库通常在局域网里，经过这个代理反而可能连不上。

没有设置时，这些请求按环境变量 `HTTPS_PROXY`、`HTTP_PROXY`、`ALL_PROXY` 和 `NO_PROXY` 决定走不走代理，都没有时直连，和加这个设置之前一样。设置了以后只用这一个代理，这些环境变量（包括 `NO_PROXY`）对元数据请求不再起作用。桌面版不读 macOS 和 Windows 的系统代理设置，从 Finder 或开始菜单打开时也没有这些环境变量，所以桌面版要走代理就在这里填。

保存时的规则：

- 去掉首尾空白之后的空字符串和 `null` 一样，表示清除设置。
- 协议只能是 `http`、`https`、`socks5` 或 `socks5h`。`socks5h` 由代理解析域名，`socks5` 在本机解析。
- 只能有协议、主机和端口，不能有路径、查询参数或者 `#` 后面的部分，末尾单独一个 `/` 可以。没写端口时，`http` 用 80，`https` 用 443，`socks5` 和 `socks5h` 用 1080。
- 不能带用户名和密码，因为这个设置会原样返回给管理员。
- 保存和返回的是整理过的写法：协议和主机名小写，末尾不带 `/`，和协议默认端口相同的 `:80`、`:443` 省略。比如 `HTTP://LocalHost:7890/` 存成 `http://localhost:7890`。

### `GET /api/admin/settings` 和 `PUT /api/admin/settings`

需要管理员的 API token。`GET` 返回 `ServerSettings`，没有保存过时 `metadata_proxy` 为 `null`。`PUT` 的请求体是完整的 `ServerSettings`，整体替换，成功返回 200 和保存后的值。请求体里没有 `metadata_proxy` 时按 `null` 处理。

错误：`metadata_proxy` 不符合上面的规则时 400 `invalid_request`，`message` 写出哪一条不对。

保存之后，元数据和封面的下一个请求就走新的代理，已经发出的请求按原来的方式完成，所以正在运行的扫描不用重新开始。asmr.one 已经选好的镜像作废，下一个请求重新探测，因为换了代理以后能连上的镜像可能不一样。

### `POST /api/admin/settings/proxy-check`

需要管理员的 API token。用请求体里的代理向 DLsite 发一个请求，看能不能连上，不保存任何东西。请求体是 `ServerSettings`，规则和 `PUT` 相同。`metadata_proxy` 为 `null` 时按上面“没有设置”的方式走环境变量，所以也能检查不设代理时连不连得上。

请求是对 DLsite 首页（`metadata.dlsite_url`）的一次 `GET`，10 秒超时，不跟随跳转，也不重试。收到任何 HTTP 回答都算连上了，包括 403 和 5xx，这时返回 200 和 `ProxyCheckResult`，样本 `contract/settings/proxy-check.json`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `url` | string | 请求的地址 |
| `status` | number | DLsite 回答的 HTTP 状态码 |
| `elapsed_ms` | number | 从发出请求到收到回答头的毫秒数 |

状态码不是 2xx 或 3xx 时，前端提示连上了 DLsite 但它回答了这个状态码，因为这多半是代理或 DLsite 那一边的问题，和连不上不是一回事。

错误：

- 400 `invalid_request`：`metadata_proxy` 不合法。
- 502 `upstream_error`：没有收到回答，比如代理连不上、DLsite 断开了连接或者超时，`message` 写出原因。
- 503 `upstream_error`：没有配置元数据来源（`metadata.source = "none"`），和 `POST /api/works/{id}/refresh` 相同。

## 导入的 JSON 输出

导入不是 HTTP 接口，但桌面版外壳和脚本要读它的输出，所以输出格式也算合同。样本在 `contract/import/`，`neo-server` 的 `contract_import` 测试按结构比对。

### `import-legacy --dry-run --json`

输出 `{"report": Report, "refusal": [string] | null}`，样本是 `preflight.json`。`report` 和导入后保存的 `import-report.json` 是同一个结构（预检时 `work_ids` 为空）。`refusal` 是导入会拒绝的理由，每条一行，没有时为 `null`。

- 路径的来源 `origin` 总是一个字符串：`given`、`default`、`guessed`，或者 `config:<config.json 的键名>`。
- `report.source.settings` 只有旧 `config.json` 白名单里的字段，见[迁移](migration.md)的“读旧的 config.json”。
- `report.old_roots[].sample` 是本地媒体库的抽查结果 `{checked, found}`，没有抽查（没找到位置、不是本地媒体库或没有作品）时为 `null`。`found` 不到 `checked` 的一半时，界面应该提醒用户这个目录可能不对。

### 导入的错误码

预检或导入失败时，`--json` 在标准输出写 `{"error": {"code", "message"}}`，和 HTTP 接口的错误同一个结构，样本是 `error.json`。`message` 给人看，是英文的完整原因链。界面按 `code` 显示自己的中文提示，不解析 `message`。

| `code` | 含义 |
|---|---|
| `source_not_found` | 选的地方没有旧 kikoeru 的数据，或者读不了 |
| `not_sqlite` | 选的或找到的文件不是 SQLite 数据库 |
| `several_sources` | 选的文件夹里有几份旧安装，要用户选一份 |
| `guessed_paths` | 有推测出来的路径，要用户确认（`accept_guessed`） |
| `unmerged_journal` | 旧库旁边有非空的日志文件，要先停旧服务，或者在副本上恢复（`recover_journal`） |
| `source_changed` | 复制期间旧库被改动，旧服务可能还在运行 |
| `not_kikoeru` | 数据库缺少 kikoeru 的表或列 |
| `too_old` | 0.6.14 以前的旧库 |
| `mysql` | 旧服务用的是 MySQL |
| `config_unreadable` | 旧 `config.json` 存在但读不了 |
| `moved_database` | 旧配置把数据库放到了这台机器上找不到的地方 |
| `target_not_empty` | 新数据目录不是空的 |
| `target_overlaps` | 新数据目录和旧数据重叠，或者就是旧数据库本身 |
| `no_library` | `--merge` 指定的地方没有库 |
| `would_refuse` | 预检发现导入会拒绝，退出码 2。`--json` 时理由在 `refusal` 里，不再另写错误 |
| `cancelled` | 收到 SIGINT 或 SIGTERM，已经回滚并清理 |
| `other` | 其他错误，比如磁盘或数据库出错 |

库函数返回的 `anyhow::Error` 里带着同一个 `ImportError`，用 `neo_server::import::error::code` 取出错误码。外壳可以把它原样交给页面。
