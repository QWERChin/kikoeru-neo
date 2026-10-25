# 技术选型

这一章列出每一层用什么，以及放弃了哪些备选。版本号以 `flake.lock`、`Cargo.lock` 和 `pnpm-lock.yaml` 为准。

## 后端

| 用途 | 选择 | 理由 |
|---|---|---|
| 语言和运行时 | Rust + tokio | 类型系统在编译期约束接口，服务以单个进程管理异步 I/O 和子进程 |
| HTTP | axum | 和 tokio、tower 生态配合最好，流式响应可以直接用 tower-http |
| 数据库 | SQLx + SQLite 或 PostgreSQL | 用 `query!` 宏的静态查询在编译期检查，两种数据库各检查一遍，迁移由 `sqlx migrate` 管理，见[数据模型](data-model.md) |
| HTTP 客户端 | reqwest | 用于 OpenList、DLsite 和其他元数据源 |
| DLsite 元数据 | serde_json | 读 DLsite 的 `product.json` 和 `product/info/ajax` 两个 JSON 接口，不解析 HTML 页面，旧代码用的是 cheerio |
| 媒体处理 | ffmpeg / ffprobe 子进程 | 转码、响度、波形和视频封装都交给 ffmpeg，Rust 只负责调度 |
| 序列化 | serde | API 类型用 `ts-rs` 导出成 TypeScript，前后端共用一份定义 |
| 日志 | tracing | 结构化日志，可交给 journald 或容器日志系统收集 |

SQLite 同时只允许一个写者，连接池和事务的具体规则见[数据模型](data-model.md)的“SQLite 设置”。

放弃的备选：

- **TypeScript 后端**：可以复用前端语言，但本项目选择 Rust 的类型约束和原生部署方式。
- **rusqlite**：同步接口更直接，但需要自己搭 worker 线程。目前没有用到 SQLx 不支持的 SQLite 扩展，所以先用 SQLx。

## 前端

| 用途 | 选择 | 理由 |
|---|---|---|
| 框架 | Svelte 5 + TypeScript | 写法最接近 HTML，运行时小，手机首屏快 |
| 应用框架 | SvelteKit，`adapter-static` | 打成纯静态文件，由后端直接托管，不需要 Node 服务 |
| 样式 | Tailwind CSS | iOS 风格要自己搭，用工具类比套现成组件库更直接 |
| 播放器 | Vidstack | 同时支持音频和视频、HLS、字幕，可以自定义界面 |
| PWA | SvelteKit 自带的 Service Worker 支持 | `@vite-pwa/sveltekit` 只支持 SvelteKit 1 和 2，manifest 是 `static/` 里的静态文件，见[前端设计](frontend.md)的“PWA” |

长列表使用分页、图片懒加载和曲目树分批绘制，见[前端设计](frontend.md#长列表)。

放弃的备选：

- **Vue 3 + Quasar 2**：组件最全，打包 App 也方便。但界面要重新设计，移动端使用网页和 PWA，桌面应用复用同一份前端，Quasar 的这两个长处都用不上。
- **Leptos 等 Rust/WASM 框架**：能全栈统一成 Rust，但这个应用的瓶颈在网络和图片，不在计算。WASM 操作 DOM 还要经过 JS，首屏也更慢。组件生态也小。
- **Video.js**：更成熟，但体积大，默认皮肤要重做。Vidstack 更容易改成 iOS 风格，以后需要时两者可以互换。

## 构建和运行

- Nix flake 统一管理 Rust 工具链、Node、pnpm、ffmpeg 和 mdBook。
- 发行物是一个 Nix 包：后端二进制加上嵌在里面的前端产物，包装脚本把 ffmpeg 和 ffprobe 放进 PATH。它不是可以随意复制到别的机器上运行的静态二进制。
- 服务器 Nix 包的目标平台是 `x86_64-linux` 和 `aarch64-linux`。桌面应用的平台与验证范围见[桌面版](desktop.md)。
