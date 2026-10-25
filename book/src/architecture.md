# 系统架构

kikoeru-neo 是一个单进程服务。它同时负责 HTTP API、前端页面、媒体流和后台任务，没有消息队列，也没有额外的 worker 进程。这样部署最简单，单用户的负载也完全够用。

## 组件

```text
浏览器（Svelte PWA + Vidstack）
        │  REST / 媒体流（Range）/ 任务快照
        ▼
┌───────────────── kikoeru-neo ─────────────────┐
│ http        路由、认证、Range、前端页面      │
│ library     作品、轨道、标签的查询与修改      │
│ storage     存储后端：local、openlist、webdav │
│ scanner     发现作品、同步轨道、抓元数据      │
│ media       ffprobe、转码、缓存               │
│ tasks       后台任务调度器                    │
│ db          SQLx、迁移、写操作串行化          │
└──────┬─────────────┬─────────────┬───────────┘
       ▼             ▼             ▼
    数据库      数据目录        ffmpeg 子进程
              （封面、缓存）
       外部：OpenList（含 115）、DLsite
```

后端代码按上面的模块分成 Cargo workspace 里的几个 crate：`neo-server`（二进制、http 和命令行）、`neo-core`（DTO、模型类型和数据库的行类型）、`neo-db`（存储层，共享业务查询集中在这里，按配置连 SQLite 或 PostgreSQL）、`neo-storage`、`neo-scanner`、`neo-media`，以及 `webdav-client`。和 OpenList 通信的 `openlist-client` 在单独的公开仓库 `chinrw/openlist-client` 里，按固定的提交依赖，见[存储后端](storage.md)。公开 DTO 和共享数据模型由 `neo-core` 定义，存储接口及 `Source`、`RemoteFile` 等类型由 `neo-storage` 定义。播放和扫描依赖这些接口，数据库操作通过 `neo-db`，各模块可用假实现单独测试。

## 请求是怎么走的

**浏览资料库**：前端请求 `GET /api/works`，后端在数据库里分页查询，返回作品列表，其中包括当前用户的收藏和播放进度。封面通过 `/api/works/{id}/cover` 单独加载，URL 里带 `cover_version`。浏览器缓存封面，每次显示时带着媒体 token 确认，封面没换就只回 304。

**播放一轨**：

1. 前端调用 `POST /api/tracks/{id}/prepare`，带上浏览器能播的格式。
2. 原格式能播，后端立即返回 `stream` 的地址。需要转码时，返回已有缓存的地址，或者创建一个转码任务并返回任务 ID，前端轮询到完成后再拿地址。
3. 播放器请求这个地址。`stream` 根据存储后端的 `access` 结果，直接按 Range 返回本地文件，或者通过 OpenList 取直链后代理转发。浏览器从不直接访问网盘。

详细规则见 [API 设计](api.md)、[存储后端](storage.md)和[媒体处理与播放](media.md)。

**扫描**：管理员在设置页点“扫描”，后端创建一个扫描任务并立即返回任务 ID。设置页读 `GET /api/tasks/{id}/events` 的事件流显示进度，事件流连续断开或者被代理挡住时改为轮询 `GET /api/tasks/{id}`，见[接口合同](contract.md)。扫描在调度器里运行，不阻塞 HTTP 请求。

## 后台任务

所有后台任务都由进程内的一个调度器持有。HTTP 请求只能创建任务或查询任务，不直接拥有任务，所以浏览器断开不影响任务继续运行。任务的状态和转换规则见[数据模型](data-model.md)的“后台任务”。

重任务（转码）和轻任务（ffprobe）各有一个并发上限，排队数量也有上限，参数见[配置与持久状态](config.md)。资源受限的服务器需要防止重任务耗尽内存和磁盘，所以并发、队列和缓存分别设限。

## 数据和文件

数据目录的结构、哪些文件要备份，见[配置与持久状态](config.md)。数据库的连接池和事务规则见[数据模型](data-model.md)的“SQLite 设置”。
