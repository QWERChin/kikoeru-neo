# 桌面版

桌面应用用 Tauri 外壳复用网页前端，并在本机模式下嵌入后端。本章说明两种运行模式、外壳权限、数据位置和打包边界。安装、首次使用、数据和日志位置见[桌面应用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/desktop.md)。两种模式的职责如下：

- **连接服务器**：窗口直接打开某台 kikoeru-neo 服务器的网址，和在浏览器里用一样，只是多了独立窗口、托盘和媒体键。
- **本机媒体库**：应用自己带着后端和 ffmpeg，在本机扫描文件夹（也可以加 OpenList 或 WebDAV 媒体库）并播放，不需要另外架服务器。

服务器版（NixOS 模块）照常发布，桌面版不替代它。尚未完成的验证见[路线图](roadmap.md)。

## 为什么选 Tauri

Tauri 的外壳是 Rust，和后端同一种语言，本机模式可以把后端直接链接进同一个进程。界面用系统自带的 WebView 渲染（macOS 是 WKWebView，Windows 是 WebView2，Linux 是 WebKitGTK），安装包比自带 Chromium 的 Electron 小一个数量级。先做 macOS 和 Windows。Linux 版 Tauri 几乎不用额外工作就能构建，所以顺带出包，但只尽力支持，验证不全时不阻塞发布。代价是各平台的 WebView 不完全一样，所以播放相关的功能要在每个平台上单独验证，不能拿 Chrome 的结果代替。

## 连接服务器模式

窗口直接加载服务器的网址，前端代码由服务器提供，而不是打包在应用里。这样桌面版的界面版本永远和服务器一致，登录、token、Service Worker 也和浏览器里完全相同，后端不需要加 CORS 或者改 API 地址。

服务器地址可以是纯 HTTP 的局域网地址，和浏览器里一样能用。macOS 的 App Transport Security 默认不让 WKWebView 打开 http 页面，外壳的检查请求能通过，窗口却加载不出来，所以 `desktop/src-tauri/Info.plist` 设了 `NSAllowsArbitraryLoads`，打包时合并进应用的 `Info.plist`。Windows 的 WebView2 和 Linux 的 WebKitGTK 没有这个限制。

外壳只做浏览器做不到的事：记住服务器地址、托盘图标、关闭窗口后继续播放、系统媒体键。媒体键先用页面已有的 Media Session，WebView 不支持的平台再由外壳补一个插件。远程页面能调用的外壳命令按 Tauri 的 capability 只开放这几个，远程页面拿不到文件系统或执行命令的权限。

## 本机媒体库模式

### 后端怎么跑

后端在应用进程里运行。`neo-server` 拆出一个库入口，外壳调用它启动服务，拿回监听的端口和一个停止句柄。服务只监听 `127.0.0.1` 的随机端口，窗口加载 `http://127.0.0.1:<端口>`，所以前端、Service Worker 和 API 都走和服务器版相同的代码路径。

ffmpeg 本来就是子进程，后端崩溃的风险不比外壳高，所以不另外拆一个 sidecar 进程。外壳通过 `neo_server::embed` 管理后端生命周期，不把服务器命令行的信号处理作为外壳退出流程。

服务器版靠 `MALLOC_ARENA_MAX=2` 压住 glibc 的内存碎片（见[测试与验证](testing.md)的“常驻内存的增长”），桌面版不设。macOS 和 Windows 不用 glibc。Linux 桌面版也没有在进程里调用 `mallopt`，需要长时间运行时应单独测量分配器碎片，不能沿用服务器包的结论。

### 数据目录和配置

本机数据目录由 `neo_server::embed::default_data_dir` 按平台环境解析，与 Tauri 外壳的配置目录和日志目录分开。具体路径由[桌面应用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/desktop.md)统一列出。数据结构遵守[配置与持久状态](config.md)，媒体库通过管理接口更新数据库，不回写配置文件。

### 网络代理

桌面客户端没有启用 reqwest 的 `system-proxy`，不读取操作系统代理设置。元数据代理由数据库设置显式选择，避免对局域网媒体库或服务器连接一并套用系统代理的例外规则。

### 登录

本机模式仍然有用户和 token，因为监听在回环地址上的端口，本机其他程序也能访问。第一次启动时外壳建一个管理员账号，随机密码存进系统钥匙串（macOS Keychain、Windows 凭据管理器、Linux Secret Service）。之后每次打开窗口，外壳通过后端的库接口直接签发一个会话 token，在页面加载前用 Tauri 的初始化脚本写进页面，用户看不到登录页。服务器版不增加任何免登录的 HTTP 接口。

### ffmpeg

ffmpeg 和 ffprobe 按目标平台作为 Tauri 的 `externalBin` 打包。后端增加配置项指定 ffmpeg 和 ffprobe 的路径，外壳把打包进来的路径传进去，不依赖 PATH。服务器版不设这两项，继续由包装脚本放进 PATH。

桌面版带启用 `--enable-gpl` 的 ffmpeg，用 libx264 转码。kikoeru-neo 使用 GPL-3.0-or-later 许可证。安装包包含项目和 ffmpeg 的许可证文本及 ffmpeg 来源信息。

## 外壳和后端的接口

本机模式下外壳把 `neo-server` 当库链接进来，只通过 `neo_server::embed` 这个模块调用后端，不碰其他模块。接口如下（Rust 签名是约定，名字以实现为准时同步改这里）：

```rust
pub struct Options {
    /// 数据目录。外壳一般传 default_data_dir() 的结果。
    pub data_dir: PathBuf,
    /// 打包进来的 ffmpeg 和 ffprobe。给了就覆盖 config.toml 里的
    /// media.ffmpeg 和 media.ffprobe，不给就沿用配置，配置也没有时找 PATH。
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    /// 前端产物。外壳传入 Tauri 为选择模式的页面嵌进去的那一份，
    /// 按相对路径（比如 index.html）返回文件，没有时返回 None。
    pub web: Option<WebSource>,
}

/// 系统的应用数据目录下的 kikoeru-neo，见“数据目录和配置”。
pub fn default_data_dir() -> Option<PathBuf>;

/// 在当前的 tokio 运行时上启动服务，等它就绪后返回。
pub async fn start(options: Options) -> anyhow::Result<Server>;

impl Server {
    /// 实际监听的端口，地址总是 127.0.0.1。
    pub fn port(&self) -> u16;
    /// 没有这个名字的用户时建一个管理员，返回是否新建。
    pub async fn ensure_admin(&self, name: &str, password: &str) -> anyhow::Result<bool>;
    /// 给已有用户签发一个 API token，和登录接口返回的完全一样。
    pub async fn session_token(&self, name: &str) -> anyhow::Result<String>;
    /// 把旧 kikoeru 的收藏、review 和进度合并进这个正在运行的库，
    /// 见[迁移](migration.md)的“合并到已有的库”。
    pub async fn merge_legacy(&self, plan: &import::plan::Plan, users: &import::merge::UserMap)
        -> anyhow::Result<import::merge::Merged>;
    /// 停止接受请求，最多等 10 秒让打开的连接发完响应，然后关闭还没发完的连接，让任务在宽限期内停下，关闭数据库，然后返回。
    pub async fn stop(self) -> anyhow::Result<()>;
}
```

几条约定：

- `start` 读数据目录里的 `config.toml`，忽略 `database.url`，始终打开数据目录里的 `neo.sqlite3`。`listen` 也被覆盖，服务总是监听 `127.0.0.1` 的随机端口。其他配置项照常验证，迁移失败、配置非法这类启动错误由 `start` 返回，外壳显示出来，不会出现半启动的服务。
- 库不安装 SIGINT 和 SIGTERM 的处理，也不初始化日志。退出由外壳调用 `stop`，日志由外壳配置 `tracing` 的订阅者，见下面的“日志”。
- `embed::start` 启动的服务里，本地媒体库没写 `watch` 时默认监听目录变化，新放进去的文件不用点扫描就会出现。`serve` 默认不监听。不需要 `Options` 字段，见[接口合同](contract.md)的“自动扫描和目录监听”。
- 媒体库的增删改走接口，不需要重启后端。改了 `config.toml` 里别的设置（比如 ffmpeg 路径）之后，先 `stop` 再 `start`。同一个数据目录同一时间只能有一个 `Server`。
- 外壳构建时不打开 `neo-server` 的 `embed-web`。Tauri 本来就要把 `web/build` 嵌进程序给选择模式的页面用，本机后端的页面也从这一份读（`desktop/src-tauri/src/web.rs`），程序里因此只有一份前端，比各嵌一份小约 7 MB。`generate_context!` 每展开一次就嵌一份，所以只在 `web::context()` 里调用它。哪些路径回退到 `index.html`、缓存头和 gzip 仍由后端按[前端](frontend.md)的“发布方式”处理，`web` 只负责按路径取文件。
- `session_token` 只是库函数，没有对应的 HTTP 接口。外壳拿到 token 后用初始化脚本写进页面，每次启动签一个新的，运行期间每天再签一个，见下面“外壳给页面的命令”里的 `use_local`。
- `merge_legacy` 也只是库函数。外壳先用 `import::session::Session::open` 找到并读出旧库，再用 `import::plan::build(&session.legacy, None, now)` 生成计划，`users` 把选中的旧用户对应到本机账号。它在运行中的服务上执行，不需要先 `stop`。
- ffmpeg 和 ffprobe 的配置项是 `media.ffmpeg` 和 `media.ffprobe`，值是可执行文件的绝对路径，不设时在 `PATH` 里找。服务器版不设这两项。`Options` 的 `ffmpeg` 和 `ffprobe` 字段传入本机模式使用的工具路径。
- 本地存储和数据目录在三个平台上各有实现，见[存储后端](storage.md)。数据目录的权限检查在 Windows 上不做：那里没有权限位，用户目录下的文件默认只有这个用户能访问。`nix flake check` 用 MinGW 工具链对 Windows 目标跑 `cargo check`。macOS 从 Linux 没有交叉工具链，它的代码路径在 Linux 上也编译和测试，macOS 原生构建由 `Desktop` 工作流运行。

## 外壳给页面的命令

页面用 Tauri 的 `invoke` 调外壳，用 `listen` 收外壳的事件。外壳是 `desktop/src-tauri` 这个 crate。哪些页面能调哪些命令由 capability 决定，按页面的来源分三类：

- **选择页**：打包在应用里的前端的 `/desktop` 页面，首次启动、切换模式和保存的模式打不开时显示。来源是 Tauri 自己的 `tauri://localhost`（Windows 上是 `http://tauri.localhost`）。
- **本机页面**：本机模式下由进程内后端提供的 `http://127.0.0.1:<端口>`。
- **远程页面**：连接服务器模式下用户填的服务器地址。

| 命令 | 参数 | 返回 | 谁能调 |
|---|---|---|---|
| `desktop_state` | 无 | `{ mode: "local" \| "server" \| null, server_url: string \| null, last_error: string \| null }`。`last_error` 是启动时保存的模式没能打开的原因，选择页显示它 | 选择页、本机页面 |
| `use_local` | 无 | 无。成功时窗口换到本机页面 | 选择页 |
| `use_server` | `{ url: string }` | 无。成功时窗口换到这个服务器 | 选择页 |
| `switch_mode` | 无 | 无。清掉保存的模式，窗口回到选择页 | 本机页面、远程页面 |
| `pick_folder` | 无 | 选中的绝对路径，用户取消时是 `null`。外壳弹出系统的选择文件夹对话框。页面拿到路径后调用 `POST /api/admin/roots` 添加本机媒体库，列出和删除也走媒体库管理接口 | 本机页面 |
| `media_state` | `{ title: string \| null, artist: string \| null, playing: boolean }` | 无。外壳用它更新托盘提示和系统的“正在播放” | 本机页面、远程页面 |
| `open_log_dir` | 无 | 无。外壳用系统的文件管理器打开日志文件夹，还不存在时先建好，见“日志” | 选择页、本机页面 |
| `import_discover` | 无 | 外壳弹出选择文件夹对话框，返回 `ImportDiscovered`：`{ pick: string, source: ImportSource }`，用户取消时是 `null`。`ImportSource` 是找到的旧数据库、封面目录和旧 `config.json`，每项是 `{ path, origin }`，`origin` 为 `"given"`、`"default"`、`"guessed"` 或 `"config:<键>"`，见[迁移](migration.md)的“找旧数据”，另有旧 `config.json` 里写的版本 `version` | 本机页面 |
| `import_preview` | `{ pick: string, accept_guessed: boolean, recover_journal: boolean }` | `ImportPreview`：`source`，`counts`（旧库的作品数、用户数，会合并的收藏、review、进度条数，以及有用户数据的作品数），`users: { name, is_admin }[]`，`old_roots`（旧配置里的媒体库，`addable` 表示是找到了位置的本地文件夹，可以放进 `add_roots`，`sample` 是抽查结果 `{ checked, found }`），`issues: { at, reason }[]`，`refusal: string[] \| null`。`refusal` 不为空时不能导入 | 本机页面 |
| `import_run` | `{ pick: string, accept_guessed: boolean, recover_journal: boolean, to_me: string[], add_roots: string[] }` | `ImportResult`：`{ works, favorites, reviews, progress, progress_kept, waiting, users_created: string[], roots_added: RootStatus[] }`。`roots_added` 是经媒体库管理接口新加的媒体库，还没扫描过 | 本机页面 |

| 事件 | 内容 | 发给 |
|---|---|---|
| `media-control` | `"play"`、`"pause"`、`"toggle"`、`"next"`、`"previous"` 之一，来自托盘菜单和系统媒体键 | 当前窗口 |

几条约定：

- 命令失败时 `invoke` 抛出一个中文的错误字符串，页面直接显示它。三个导入命令例外：它们抛出 `ImportFailure`，即 `{ code, message }`。`code` 是[接口合同](contract.md)“导入的错误码”里的一个，页面按它写中文提示，外壳自己的错误（比如没先选文件夹、已经不在本机模式）是 `other`。`message` 是完整的原因，导入库给的部分是英文，页面放在提示下面。`import_run` 在 `refusal` 不为空时以 `would_refuse` 失败，`message` 每行一条理由。
- 模式和媒体控制的类型定义在 `neo-core::dto`（`DesktopState`、`DesktopMode`、`MediaControl`），外壳直接用它们，前端用 `neo-typegen` 生成的 TypeScript 类型，和 API 类型一样不手写。导入命令的类型也在 `neo-core::dto`（`ImportDiscovered`、`ImportPreview`、`ImportResult`、`ImportFailure` 和它们用到的类型），外壳把导入库的报告转成它们，只留页面要显示的部分。完整的报告仍然是 `import-report.json` 的结构，只在命令行的 `--json` 里输出。
- `use_local` 第一次调用时建本机管理员：随机密码存进系统钥匙串，然后启动后端，签发会话 token，在页面加载前写进 `localStorage` 的 `kikoeru-neo.token`（前端读 token 的同一个键）。初始化脚本先比对 `location.origin`，只在本机后端的页面里写，跳到别的地址时不会带过去。以后每次启动都签一个新的。外壳运行期间每天再签一个新的写进页面的 `localStorage`，初始化脚本只在已存的 token 比自己的更早过期时才覆盖，所以刷新页面不会换回旧 token。前端在本机模式下收到 401 时，先重新读一次 `localStorage` 里的 token，和内存里的不同就换上重试，相同才去登录页。
- `use_server` 只接受 `http` 和 `https` 地址，先请求一次 `/api/health` 确认是 kikoeru-neo 服务器再保存。这个请求被重定向时，只接受同一主机（比如从 http 换成 https），保存和固定的是重定向后的地址。启动时也先这样检查一次保存的服务器，连不上就回到选择页并给出原因。远程页面自己登录，外壳不给它注入 token。
- 远程页面只能调 `media_state`、`switch_mode` 和拖动自己的窗口，拿不到文件系统、对话框、执行命令和其他命令。`switch_mode` 最多让服务器把窗口送回选择页，读不到也改不了本机的任何东西，而连错服务器或者忘了密码的用户停在登录页上，需要一个不靠托盘菜单的出口。
- macOS 上窗口用 Tauri 的 Overlay 标题栏并隐藏标题，页面一直画到窗口顶边，红绿灯浮在侧栏上，和系统自带的应用一样。外壳在每个页面加载前设 `window.__KIKOERU_NEO_TITLE_BAR__ = 'overlay'`，页面据此留出标题栏的高度。页面盖住了真正的标题栏，所以拖动窗口和双击缩放要靠页面上的 `data-tauri-drag-region`，三类页面都有 `core:window:allow-start-dragging` 和 `core:window:allow-internal-toggle-maximize` 这两个权限。Windows 和 Linux 保留系统标题栏，不设这个变量。
- 窗口不会离开它打开时的来源：选择页只留在打包的页面里，本机页面只留在本机后端的那个端口，远程页面只留在那个服务器。跳到别的来源的导航会被拦下，所以页面没法把窗口带到另一个仍然有权限的来源。本机页面和远程页面的 capability 只写当次打开的那个窗口的标签，标签不会重复使用。Tauri 不会撤销已经加上的 capability，如果写成所有 `main-*` 窗口，之后在同一来源打开的远程窗口（比如服务器地址恰好是之前本机后端用过的端口）也会拿到本机页面的命令。
- 模式和服务器地址存在系统的应用配置目录下的 `desktop.json`，不和数据目录放在一起，因为连接服务器模式没有数据目录。
- 同一个用户只运行一个外壳进程，再次打开只会把已有的窗口显示出来，因为两个本机后端会争用同一个数据目录。
- 关窗口只是隐藏，播放继续，托盘菜单的“退出”才结束程序。托盘的“显示窗口”和 macOS 上点 Dock 图标都会把它显示出来。退出时本机后端按宽限期停下转码再关闭数据库。
- 导入旧库只走[迁移](migration.md)的“合并到已有的库”：旧库的收藏、review 和播放进度合并进本机库，作品、封面和元数据由本机自己扫描得到。`import_preview` 和 `import_run` 只接受最近一次 `import_discover` 对话框里选的文件夹，所以页面没法让外壳读用户没选过的路径。每次都按 `pick` 重新找旧数据、复制一份快照再读，所以预览和导入之间旧库变了也不会写进过期的数据。`accept_guessed` 确认推测出来的路径，`recover_journal` 在旧库旁边有未合并的日志时让外壳在副本上恢复，两者默认都该是 `false`，失败信息会说明什么时候需要打开。`to_me` 列出合并进本机账号的旧用户，其他旧用户各自建一个没有密码的同名账号。`add_roots` 列出要加进本机媒体库的旧媒体库名字，只能是 `report.old_roots` 里 `kind` 为 `"local"` 且找到了 `path` 的项。外壳用 `POST /api/admin/roots` 添加它们，显示名是旧名字，`id` 是新生成的 `local-<12 位十六进制>`，路径已经是某个本地媒体库的跳过。添加立刻生效，不重启后端，扫描由页面另外发起。OpenList 媒体库由用户在设置里用媒体库管理接口添加。用户数据先合并，再加媒体库：合并之后的步骤失败时，错误信息会说明用户数据已经合并了。导入过程中切换模式会等它结束。
- 系统钥匙串不可用时（比如 Linux 上没有运行 Secret Service），外壳只记一条警告，照样启动：登录靠外壳签发的 token，不需要这个密码。

## 前端怎么用这些命令

前端还是同一份 SvelteKit 产物，桌面版的界面都在 `web/src/lib/desktop/`、`web/src/lib/components/desktop/` 和 `web/src/routes/desktop/` 里。在普通浏览器里这些界面都不出现，已有的页面和以前完全一样，打开 `/desktop` 会跳回首页。

- **认出外壳**：Tauri 在它加载的每个页面里先跑自己的初始化脚本，设好 `window.isTauri`，本机页面和远程页面也有，所以前端用它判断是不是在外壳里。在外壳里时，来源是 `tauri://localhost` 或 `http://tauri.localhost` 的是选择页。其他页面调一次 `desktop_state`：能调就是本机页面，被 capability 拒绝就是远程页面。只看 capability 而不看它返回的 `mode`，因为外壳切换模式时先打开新窗口再保存模式。页面只在打开时调这一次，答案留在 `desktop.state` 里。
- **选择页**：打包的前端只有 `/desktop` 这一页，别的路径都跳到这里。页面读 `desktop_state`，有 `last_error` 时放在最上面，`server_url` 预先填进地址框。“连接服务器”调 `use_server`，地址没写 `http://` 或 `https://` 时前面补上 `http://`，服务器要 https 会自己重定向，外壳接受同一主机的重定向。检查服务器是外壳的事，选择页的来源和服务器不同源，服务器又不开 CORS，页面自己请求不到 `/api/version` 或 `/api/health`。“使用本机媒体库”调 `use_local`。两个命令成功时外壳会换掉这个窗口，所以页面一直停在“正在连接…”或“正在启动本机媒体库…”，失败时显示外壳的错误并恢复按钮。有 `last_error` 时，提示下面有一个“打开日志文件夹”按钮，调 `open_log_dir`，因为本机媒体库为什么没能启动只写在日志里。
- **本机页面**：外壳在页面加载前写好 token，所以直接进资料库，不出现登录页。设置页多一栏“桌面应用”，写着模式和保存的服务器地址，有“打开日志文件夹”（调 `open_log_dir`）和“切换模式”，确认后调 `switch_mode`。远程页面的这一栏没有“打开日志文件夹”，它不能让外壳打开本机的文件夹。账号一栏没有“退出登录”，因为本机账号的密码只在钥匙串里，退出之后没法再登录。万一还是到了登录页，页面上说明这是自动登录的，让用户从托盘退出后重新打开，下面也有“切换模式”。本机媒体库文件夹走管理员媒体库接口和 `pick_folder`，那套界面和服务器版的设置页共用，不在这里。
- **从 kikoeru-express 导入**：只在本机页面、只对管理员显示，在设置页的“桌面应用”下面。“选择旧数据目录…”调 `import_discover`，对话框关掉时什么也不变。找到旧数据后列出数据库、封面和 `config.json` 的路径和来源（推测出来的标红），以及旧版本，然后马上调 `import_preview`，两个确认项都先是 `false`。预览列出旧库的作品数、用户数、收藏、评价和进度条数，旧用户默认勾选管理员合并进本机账号，旧媒体库里能加的本地文件夹默认勾选，抽查时找到的作品不到一半的写明可能不对、不默认勾选，OpenList 和找不到位置的媒体库不能勾选并写明原因，会被跳过或改动的记录折叠起来。`refusal` 不为空时列出理由，“导入”按钮禁用。`import_preview` 失败时按错误码写中文提示，下面是完整原因：`guessed_paths` 给出“使用推测出来的路径”，`unmerged_journal` 给出“在副本上恢复日志”，勾上后“再检查一次”，勾选会一直带到 `import_run`。“导入”调 `import_run`，失败时提示同样按错误码写，勾选保留，可以再试。成功后显示合并了多少作品的用户数据、收藏、评价、进度，保留了几条本机更新的进度，几部作品还要扫描，新建了哪些本机账号和新加的媒体库。每次导入之后都重新读取媒体库列表，因为外壳在合并之后一个一个地加媒体库，失败的导入也可能已经加了一部分。新加的媒体库由用户在列表里扫描。流程写在 `web/src/lib/desktop/legacy-import.svelte.ts`，错误码的中文在 `import-errors.ts`。
- **选文件夹**：媒体库管理的“选择文件夹…”经 `web/src/lib/desktop/folder-picker.ts` 调 `pick_folder`，它用同一个 `desktop.detect()` 判断是不是本机页面，所以一个页面只调一次 `desktop_state`。
- **远程页面**：和浏览器里一样自己登录。登录页的表单下面写着当前连接的服务器，并有“切换模式”，所以连错了服务器不用登录就能回到选择页。设置页的“桌面应用”写着模式和当前服务器，也有“切换模式”。两处共用 `SwitchModeButton`，确认后调 `switch_mode`。
- **macOS 标题栏**：`overlayTitleBar()` 读外壳设的变量，为真时根布局给 `<html>` 加上 `data-title-bar="overlay"`，CSS 变量 `--title-bar-h` 从 0 变成 40px，并在窗口顶部放一条这么高的拖动条。系统标题栏的高度随 macOS 版本变化，所以外壳用 `traffic_light_position` 把红绿灯固定在这 40px 里，而不是假定标题栏一定是 28px。宽窗口里侧栏的背景延伸到顶边，可以滚动的部分从标题栏下面开始，主栏也从标题栏下面开始，拖动条是透明的。模态对话框打开时页面的其他部分都不响应鼠标，所以每个对话框里也放了一条同样的拖动条（`TitleBarDrag`）。窄窗口里整页往下推，拖动条用页面背景色，滚上去的内容不会露在红绿灯后面。播放页、收藏提示和悬浮视频小窗把 `--title-bar-h` 和 `safe-area-inset-top` 取大的那个。只认外壳的变量而不认 macOS 的 User-Agent，是因为旧版外壳还有系统标题栏，不该多留空白。反过来，新版外壳连到还没升级前端的服务器时，页面没有留空，红绿灯会压在搜索框上，也拖不动窗口，所以服务器和桌面版要一起升级。
- **托盘和媒体键**：本机页面和远程页面在播放器的轨道、社团或播放状态变化时调 `media_state`，内容和 Media Session 一样，没变时不重复发。收到 `media-control` 事件时对播放器执行同名的操作。系统媒体键继续走页面的 Media Session。
- **续签的 token**：本机页面收到 401 时先读一次 `localStorage` 里的 token，和发出请求用的不同就换上，再把这个请求重发一次，任务进度流也一样。还是 401，或者存的就是失败的那个，才照常清除登录状态去登录页。换上续签的 token 不算换账号，所以播放不会停。

## 日志

外壳负责进程级 tracing 初始化，嵌入后端不重复初始化。外壳同时写 stderr 和 Tauri 的 `app_log_dir()`，避免无控制台的安装包丢失错误信息。文件以 UTC 日期轮转，打开写入器时保留最新 7 个文件，文件不可写时回退到 stderr 并告警。默认日志级别是 info，环境过滤器可以覆盖它，后端文件中的 `log.level` 不控制外壳。

日志入口只对选择页和本机页面开放，远程页面不能调用本地文件管理器。具体位置和排查操作见[桌面应用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/desktop.md)。

## 构建和发布

Linux 版由 flake 构建：`nix build .#desktop`。它在自己的 Cargo workspace 里（`desktop/src-tauri`），所以服务端的构建和测试不会编译 Tauri 和 WebKitGTK。包里带上了 WebKitGTK 播放音视频要用的 GStreamer 插件和托盘需要的 libayatana-appindicator。包装脚本还把 nixpkgs 的 `ffmpeg-headless` 放进 PATH，本机模式的后端按名字找到它，用户不用另外装 ffmpeg。GStreamer 的 libav 插件本来就依赖 `ffmpeg-headless` 的库，所以只多出两个可执行文件，不到 1 MiB。这里不用服务端的精简版，因为那会再带进一份不同的库。`nix flake check` 还会对 Windows 目标跑一遍外壳的 `cargo check`。

macOS 和 Windows 的安装包由 GitHub Actions 的 `Desktop` 工作流（`.github/workflows/desktop.yml`）构建，在推送 `v*` 标签时运行，也可以在 Actions 页面手动运行。它不在 pull request 上跑，因为安装包只在发布时需要，平常的改动由 CI 里的 `desktop` 和 `desktop-windows` 检查覆盖。

- 前端先在 GitHub 托管的 Ubuntu runner 上用 `nix build .#web` 构建，所以安装包里的前端和字幕回退字体与服务器版完全相同。
- macOS 只出 Apple Silicon（`aarch64-apple-darwin`）的 `.app` 和 `.dmg`，Windows 出 NSIS 安装包。Intel Mac 要再构建一次，还要另一份 ffmpeg，先不做。
- ffmpeg 和 ffprobe 由 `scripts/desktop/fetch-ffmpeg.sh` 下载，地址固定到版本，并核对 sha256，文件变了构建就失败。macOS 用 Martin Riedl 的静态构建，Windows 用 Gyan Doshi 的 essentials 构建，两者都是 GPL v3。它们作为 Tauri 的 `externalBin` 装在主程序旁边，外壳在那里找到它们，见上面的“ffmpeg”。
- 安装包里的 `licenses/` 有 `FFMPEG.txt`（来源、源码地址和 GPL v3 全文）和 kikoeru-neo 自己的许可证。macOS 上在 `.app/Contents/Resources/licenses`，Windows 上在安装目录下。
- 这些只在发布时需要的设置放在 `desktop/src-tauri/tauri.bundle.conf.json`，构建时用 `--config` 合并进去。`nix build .#desktop` 和平时的 `cargo build` 不读它，所以不需要先下载 ffmpeg。
- 每个平台构建完都跑一次冒烟测试：直接运行 `.app` 里的程序，Windows 上先静默安装再运行装好的程序，参数是 `--smoke <报告文件> [<媒体库>]`。它不开窗口，运行打包的 `ffmpeg -version` 和 `ffprobe -version`，用它们启动进程内后端，再请求 `/api/health` 和首页。给了媒体库时还会扫描它并准备每条轨道，见[测试与验证](testing.md)的“桌面版验证”。结果写进报告文件，退出码表示成败。
安装包使用的签名配置、获取方法与自动更新限制见[桌面应用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/desktop.md)。工作流产物不等同于已发布到 Releases 的安装包。

## 要在每个平台验证的事

能自动做的检查在 `Desktop` 工作流里，要手动做的列在[测试与验证](testing.md)的“桌面版验证”。下面是还要确认的行为。

- 连接服务器模式下打开纯 HTTP 的局域网地址，包括 IP 地址和主机名两种写法，macOS 上尤其要确认。
- 后台播放、锁屏或通知中心的播放控制、媒体键。
- WebView 能直接播放哪些音频格式，`canPlayType` 的结果会改变 `prepare` 的判断，见[媒体处理与播放](media.md)。
- 视频播放需要验证：原生 HLS 还是 hls.js、全屏、画中画、JASSUB 的 Web Worker 和 WebAssembly 在 WebView 里能否运行。
- 页面的内容安全策略（`vite.config.ts` 生成的 meta）不拦 WebView 里的东西。Windows 的 WebView2 由 `Desktop` 工作流的 WebDriver 测试自动检查。Linux WebKitGTK 有单独的本地 WebDriver 运行方式，不在该工作流中。测试要求：选择页、本机模式和连接服务器模式的资料库、播放页、四种格式的播放和 ass 字幕上任何一次 CSP 报错都让测试失败。macOS 的 WKWebView 没有 tauri-driver，要在真机上打开开发者工具，按同样的步骤看控制台里有没有 CSP 报错。
- 本机模式下关闭窗口、退出应用、系统休眠时，进行中的转码任务和进度上报的表现。
- 日志文件写进了[桌面应用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/desktop.md)列出的文件夹，“打开日志文件夹”在访达、资源管理器和 Linux 的文件管理器里打开的是这个文件夹。
- Windows 上 neo-media 和 Unix 有三处不同，要在真机上确认：
  - 缓存盘的剩余空间还没有测量，`cache.min_free_bytes` 的检查跳过，只靠 `cache.max_bytes` 限制缓存大小。
  - 取消或超时时没有 SIGTERM，直接结束 ffmpeg，不给它收尾的 5 秒。
  - Windows 没有 Unix 的文件描述符输入路径，当前实现已通过 `Input::File` 把本地路径交给 ffmpeg。真机仍需验证路径重开、特殊字符和播放行为，不能把实现存在当成运行验证。
