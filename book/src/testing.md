# 测试与验证

测试使用合成作品、临时数据库和本地假服务，验证数据完整性、API 合同、媒体处理及用户流程。运行测试不需要已有媒体库或外部服务账号。构建成功、自动测试通过和真机验证是不同的证据，未覆盖的平台行为必须单独记录。

## 运行检查

在仓库根目录运行：

```sh
nix flake check
nix build .#book --no-link
```

完整的 Rust、浏览器和虚拟机检查在 `x86_64-linux` 上定义。`aarch64-linux` 的检查集合较小，不能把它的 `nix flake check` 当成同等运行覆盖。只跑相关检查时可以使用：

```sh
nix build .#checks.x86_64-linux.rust-test --no-link
nix build .#checks.x86_64-linux.rust-test-pg --no-link
nix build .#checks.x86_64-linux.e2e --no-link
nix build .#checks.x86_64-linux.e2e-mock --no-link
```

Nix 可以复用已成功的检查产物，不保证每次都执行测试。需要重跑某项时加 `--rebuild`。本地快速检查在 `nix develop` 环境中运行：

```sh
cargo test --workspace --locked
scripts/sqlx-prepare.sh --check
cd web
pnpm install --frozen-lockfile
pnpm check
pnpm test
```

直接运行 Cargo 时没有嵌入前端，不能替代下面的生产包检查。修改查询或迁移后要按[构建与部署](deployment.md)重新生成两个后端的 SQLx 离线元数据。

## 检查矩阵

| 检查 | 覆盖范围与边界 |
|---|---|
| `rust-lint` | 检查 rustfmt、带和不带 `embed-web` 的 Clippy，以及 SQLite、PostgreSQL 两份 `cargo sqlx prepare --check`。 |
| `rust-test` | 运行根 workspace 的 Rust 测试，打开 `embed-web` 检查真实前端产物，媒体测试使用 ffmpeg 和 ffprobe。桌面外壳是独立 workspace，不在其中。 |
| `rust-test-pg` | 在临时 PostgreSQL 上运行 `db`、`legacy_state`、`pg_schema`、`db_copy`、`scan`、`metadata`、`password_cleanup` 测试目标，并包含忽略的用例。`password_cleanup` 用延迟触发器暂停媒体库增删改的真实提交，验证另一实例的回收等待后保留新引用。它不是全部 Rust 测试的 PostgreSQL 重跑。 |
| `neo-typegen` | 从 Rust DTO 生成 TypeScript，并用严格类型检查验证合同样本。 |
| `web` | 构建静态前端，运行 `svelte-check` 和 Vitest，拒绝混入假数据的生产产物。 |
| `neo-server` | 构建嵌入前端的发布包。包本身的 `doCheck` 为 `false`，Rust 测试与 lint 由独立检查执行。 |
| `e2e` | Playwright 访问临时数据上的生产包，覆盖认证、资料库、扫描、播放、进度和恢复流程。 |
| `e2e-mock` | Playwright 访问假数据开发服务器，覆盖页面状态、表单、权限显示、错误交互、布局和桌面外壳模拟。不能代替真实后端集成。 |
| `ffmpeg` | 检查精简版 ffmpeg 所需的解码器、编码器、封装器、滤镜和协议，解封装器清单与输入白名单一致。 |
| `docker-image` | 不启动 Docker，检查镜像配置、权限、入口、卷、健康检查脚本和镜像大小。 |
| `docker-rootfs` | 检查 Dockerfile 使用的闭包归档，验证私有卷为空、uid/gid 1000、0700，以及工具链接和 `/tmp` 权限。 |
| `dockerfile` | 将最终阶段的入口、参数、用户、环境、端口、卷、健康检查和标签与共享 Nix 配置比较，并验证字段漂移会被发现。 |
| `neo-server-static` | 仅在 x86_64 Linux 上检查 musl ELF 没有 INTERP 或 NEEDED，在临时数据目录创建管理员，启动后取得 `/api/ready` 和嵌入首页的 200。不测试媒体转码。 |
| `systemd` | 安装仓库中的普通 Linux 单元和静态包，通过默认 PATH 使用 `/usr/bin` 的媒体工具，验证管理员、就绪、0700 所有权、真实响度与波形、停服备份，以及移除 ffmpeg 后仍就绪并告警。不证明所有发行版的 systemd 或媒体包都兼容。 |
| `docker-migrate` | 用 bats 和假 Docker 命令检查旧容器迁移脚本的挂载、权限、终端与失败清理行为。 |
| `docker` | NixOS VM 中运行真实 Docker，用合成旧容器验证迁移、启动、登录和中断清理，不启动源容器。 |
| `nixos` | 启动 NixOS 模块，检查权限、管理员、备份与恢复、重启后密钥，以及本机 PostgreSQL 配置和数据库复制。 |
| `windows`、`desktop-windows` | 用交叉工具链检查 Windows 代码能编译，不证明 Windows 运行行为。 |
| `desktop` | 构建 Linux 桌面应用及其依赖，不替代 WebView 和系统集成验证。 |
| `ghcr-release` | 用假镜像工具检查版本标签、重复发布和失败处理，不实际推送镜像。 |
| `github-release` | 用假 API 检查新 Release、相同资产重试、checksum 冲突、部分上传恢复和查询失败，不实际发布。 |
| `release-ci-gate` | 用临时 Git 仓库和假 API 检查主线归属、CI 结果与等待、纯文档差异，以及异常响应和分页，不访问 GitHub。 |
| `book` | 构建 mdBook，不验证代码行为，也不能代替链接和文字审查。 |

GitHub Actions 的 `CI` 工作流在托管 runner 上按检查分别运行，检查集合来自 flake 的 `x86_64-linux` 属性。只改书籍、用户指南、贡献指南、README 或 `AGENTS.md` 时，工作流会跳过，因此文档改动仍需在本地构建书籍。构建沙箱可能没有 `/usr/bin/env`，测试脚本使用 `#!/bin/sh`。需要写用户目录的检查必须指向临时目录，不依赖 `/homeless-shelter` 可写。

相关构建输入变化时，CI 还在 `ubuntu-latest` 上从头构建服务和开发容器。`scripts/docker-smoke.sh` 使用临时配置和新命名卷，等容器 healthy 且 `/api/ready` 为 200 后才创建合成管理员。管理命令也会运行迁移，而 SQLite 的迁移锁不让两个进程排队，所以首次启动的迁移结束前不能在另一个进程里打开数据库。`docker-smoke` 检查用假 docker 命令验证这个顺序和失败时的清理。`scripts/devcontainer-smoke.sh` 在断网容器中验证 Bash 的三种启动方式、`sh` 和直接 `docker exec cargo`，并用 mold 编译一个小程序。它们不证明 aarch64 或 IDE 远端服务已实测，IDE 与原生 ARM64 验证需另行记录。

## Rust 与合同测试

合同测试为每个 `contract/` 样本构造 DTO，比较完整序列化值并检查反序列化。字段类型、空值、具体值和安全整数边界都属于检查范围。前端必须使用生成的类型，不手写响应类型。

数据库测试用临时库覆盖迁移、事务、去重、缺失标记、用户状态和复制。`pg_schema` 比较两种数据库的结构，但不完整比较 `CHECK` 表达式。`progress_order`、`root_media` 和 `folder_cover` 等可切换后端的目标没有列入当前 `rust-test-pg` 命令，这是覆盖缺口。

调度器的 `tasks` 集成测试也支持切换后端，但 SQLite 专用触发器、文件重开和 HTTP 测试夹具仍使用临时 SQLite。验证 PostgreSQL 入队提交结果不确定时，在开发环境里用 `initdb` 和 `pg_ctl` 启动只监听 Unix socket 的临时服务器，把 `KIKOERU_TEST_PG` 指向它，并运行：

```sh
KIKOERU_TEST_BACKEND=postgres cargo test --locked -p neo-server --test tasks
KIKOERU_TEST_BACKEND=postgres cargo test --locked -p neo-server --lib tasks::tests -- --include-ignored
KIKOERU_TEST_BACKEND=postgres cargo test --locked -p neo-db --test db
```

单元测试中的故障注入只在测试构建启用。它完成真实事务的提交或回滚后返回连接错误，验证原任务能继续执行、去重不会运行第二份任务，以及未提交的任务不会执行。PostgreSQL 专用用例默认忽略，以上命令显式启用它们，当前 `rust-test-pg` 检查尚未包含这两个调度器测试目标。

进程级测试实际启动 `serve` 和 `user` 命令，通过回环 HTTP 检查首次管理员初始化、密码重设、删除用户、重启后 token、密钥更换、非法配置和失败退出。API 测试直接调用 axum 路由，媒体测试覆盖 Range、HEAD、错误状态、取消、缓存发布和临时文件清理。

导入测试在临时目录创建旧版本的合成 SQLite 表、配置和封面。修改导入或数据模型时，应覆盖支持版本、字段单位、Windows 路径、非法或重复记录、进度歧义、源目录不变、失败回滚和导入报告。不能要求贡献者使用某一份私人库来通过验收。

## 浏览器测试

先构建包含前端的服务，再在开发环境中运行：

```sh
nix build
nix develop -c sh -c 'cd web && KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e'
nix develop -c sh -c 'cd web && pnpm e2e:mock'
```

浏览器来自 nixpkgs 的 `playwright-driver.browsers`，开发环境设置 `PLAYWRIGHT_BROWSERS_PATH`，不需要运行浏览器下载命令。`@playwright/test` 必须与锁定的驱动版本一致。每个浏览器项目使用自己的合成用户，避免进度相互影响。

| 用例位置 | 验证的行为 |
|---|---|
| `web/e2e/app.spec.ts` | 登录、分页、封面状态、退出、深链接和真实 404。 |
| `web/e2e/login-session.spec.ts` | 晚到的媒体凭据响应不恢复已退出会话，不覆盖新账号，媒体凭据交换失败仍完成登录，以及跨标签页凭据隔离。 |
| `web/e2e/progress-order.spec.ts` | 页面真实关闭后的报告乱序提交，旧报告不覆盖新位置，重新打开后恢复新进度。 |
| `web/e2e/metadata.spec.ts` | 本地假 DLsite 正常、失败与挂起时的扫描，手动元数据保护、取消、SSE 断开和扫描队列调整。 |
| `web/e2e/roots.spec.ts` | 界面添加与删除媒体库，假 WebDAV 上认证失败后显示“仍然保存”，修正密码后保存、扫描及 Range 读取。点“仍然保存”提交 `skip_check: true` 由 `web/e2e-mock/roots.spec.ts` 覆盖。 |
| `web/e2e/video.spec.ts`、`video-matrix.spec.ts` | 本地和假 OpenList 的直放、HLS、拖动、字幕、媒体 token 过期、缓存淘汰和取消。 |
| `web/e2e/insecure.spec.ts` | 非安全上下文下的核心流程与缺失浏览器 API 的处理。 |
| `web/e2e-mock/` | 收藏、搜索、评价、设置、媒体库表单、代理、主题、下载、长列表、响度与字幕，以及模拟桌面命令和导入。 |

`process_m4.rs` 用本地假 DLsite 和合成 WAV 检查服务端的扫描与元数据规则，`process_m6.rs` 用 ffmpeg 现场生成视频，覆盖本地和假 OpenList 的准备、分片鉴权、取消、重启与缓存淘汰。文件名中的编号是现有测试文件路径，不表示仍待实现的产品阶段。

### 进度乱序回归

测试代理收齐合成用户的 PUT 请求后先扣住它们。测试产生位置较早的 A，再真实关闭页面产生序号更大的 B。先放行 B，等后端返回 204，再放行 A，直接读取后端确认位置、轨道、分组与完成状态不变。之后才新开页面检查恢复，不能靠页面补写掩盖旧报告覆盖。

```sh
nix build
nix develop -c sh -c 'cd web && KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e progress-order.spec.ts --project desktop --project phone'
```

### 纯 HTTP

测试用 `http://kikoeru-lan.test` 作为页面来源，把请求代理到回环服务，避免浏览器把 localhost 当成安全上下文。无需真实网卡，构建沙箱也能复现。`insecure` 跑 Chromium 核心流程，`insecure-firefox` 因打包浏览器缺少部分媒体解码器，只跑非媒体用例。

```sh
nix develop -c sh -c 'cd web && KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e --project insecure --project insecure-firefox'
```

`KIKOERU_E2E_INSECURE_CONTROL=1` 用于安全上下文对照，不能当作产品修复。需要真实网络测试时可设置 `KIKOERU_E2E_LAN_IP` 为测试机地址，不依赖固定网卡名。

## 媒体格式与缓存验证

用 ffmpeg 生成静音或测试图案，不提交私人媒体。音频矩阵覆盖 MP3、WAV、FLAC、M4A、Ogg 以及需要 AAC 回退的格式。视频矩阵覆盖 H.264 + AAC 的 MP4 直放、MKV 与带 B 帧 AVI 的转封装，以及 MPEG-4 + MP3 的 HLS 转码。

每种样本分别验证准备、非零位置 Range、拖动、结束、取消和重新请求。缓存实验将预算设为少数合成结果的大小，检查整组淘汰、正在使用的结果保护、淘汰后的重新生成，以及中断后临时目录和子进程的清理。格式扩展名和 `canPlayType` 的回答不能代替实际解码验证。

## 手机播放验证

浏览器标签页和主屏幕 PWA 分别验证，记录设备、系统、浏览器版本和访问方式。合成媒体应包含可直放音频、需要回退的音频、MP4、HLS 和 ASS 字幕。

- 首次播放、反复拖动、切轨、刷新后恢复。
- 切到后台与锁屏后继续播放，并自动进入下一轨。
- 系统播放界面与耳机按钮控制，以及实际声音输出。
- 长时间暂停后继续，网络切换和服务器短暂不可用后恢复。
- 媒体 token 过期、远端直链过期和缓存淘汰后恢复原位置。

### iOS 模拟器

`.github/workflows/ios.yml` 是手动触发的独立工作流，不在每个拉取请求上运行。它在 macOS 上启动服务，使用 `scripts/ios/make-library.sh` 生成媒体，经本地 TLS 代理供 Safari 和主屏幕网页访问。XCUITest 驱动界面，结果、截图和录屏由工作流保存。模拟器可验证界面和服务收到的进度，不能证明真实设备的耳机按钮、锁屏界面或声音输出。

Android 模拟器同样不能替代真机的系统媒体控制验证。Android 与 iOS 真机的上述清单仍是未完成的验证工作，不应记成已通过。

## 桌面版验证

`Desktop` 工作流构建 macOS 与 Windows 安装包，并对安装后的程序运行 `--smoke <报告文件> <媒体库>`。`scripts/desktop/make-fixture.sh` 使用打包的 ffmpeg 生成 MP3、WAV、MP4 和 MKV。冒烟模式不打开窗口，检查打包的工具、进程内后端、扫描、准备和读取媒体。

Windows 另有 WebView2 WebDriver 检查。Linux 的 WebDriver 测试可以单独运行，但不属于这个工作流。macOS 没有同等的 tauri-driver 覆盖。必须在各平台补充验证连接服务器和本机模式、HTTP 地址、媒体键、后台播放、休眠、窗口关闭、日志入口、HLS、画中画和 JASSUB，见[桌面版](desktop.md#要在每个平台验证的事)。

## 资源预算

资源测量用合成数据和固定负载，记录硬件级别、构建参数、样本规模、并发数、缓存状态和采样方法。单机结果不作为其他机器的性能保证。

### 资源基准脚本

```sh
nix build .#neo-server
env -u KIKOERU_CONFIG -u KIKOERU_DATABASE_URL -u KIKOERU_LISTEN \
  nix develop -c python3 scripts/bench.py --bin ./result/bin/kikoeru-neo > bench.json
```

`bench.py` 建临时数据目录，生成合成作品和媒体，测空闲、并发播放、扫描和转码时的资源与延迟。可用 `--works`、`--tracks`、`--streams`、`--load-seconds` 改变负载，`--keep` 保留测试目录。比较结果时保持参数和缓存条件相同。转码缓存也存放波形，因此淘汰计数只比较 AAC 产物，容量计数仍包含所有缓存文件。脚本报告包含主机名与本地路径，公开报告时仅保留硬件级别、合成负载和指标。

### CI 的内存预算

先构建一次目标检查，再运行 `scripts/check-memory.py e2e-mock`，脚本重建该检查并采样构建用户进程的 RSS。共享页会重复计数，页缓存与 tmpfs 不包含在 RSS 中，不能把它当成整个 cgroup 的精确峰值。测量时控制其他负载，分别记录成功、超时和资源压力，假数据浏览器测试的基线仍待补充。

### 浸泡测试

`scripts/soak.py` 对正在运行的隔离测试服务模拟多个听众，执行 Range 读取、拖动、换轨、响度和波形请求、强制转码，以及可选的中途重扫。只应指向可丢弃的合成媒体库，不能把它描述成只读检查，因为它会创建任务并可触发扫描。

默认通过服务的用户级 systemd 单元查找进程，用 `/proc` 采样资源。没有用户总线时，可用 `--pid PID --log-file FILE` 代替 `--unit`，本轮新建的服务日志仍用于发现限流。重复 `--root` 会把监听者分配到不同媒体库，`--seed` 为每个工作线程固定独立的随机序列，`--cooldown-seconds 600` 会在停止负载后继续采样十分钟。报告包含整个负载期间的 API 探针 p50、p95、p99、服务器及子进程 CPU 时间和主机负载平均值。RSS 与 swap 分别记录，同时保留两者之和，避免把换出误认为分配器释放了内存。

```sh
python3 scripts/soak.py --url http://127.0.0.1:18900 --user admin \
  --password-file /tmp/neo-synthetic/password \
  --data-dir /tmp/neo-synthetic/data --unit neo-synthetic.service \
  --root local --rescan-root cloud --listeners 3 --hours 1
```

这里的路径、单元和媒体库 ID 是合成测试环境的示例，运行前须自行创建对应服务、账号和媒体库。无时长轨道的强制转码按服务端 AAC 输出的 192 kbit/s 控制读取节奏，有时长的轨道仍按实际返回的文件大小和时长计算读取速率。

### 常驻内存的增长

远端转发会在多个运行时线程上产生短期分配。排查 RSS 增长时，应比较分配器仍在使用的内存、空闲 arena、句柄和子进程数量，区分碎片与泄漏。NixOS 模块和 Docker 镜像设置 `MALLOC_ARENA_MAX=2` 限制 glibc arena 数量，这不替代缓存预算或泄漏检查。

musl 单文件包用 jemalloc 代替 musl 自带的 mallocng，由 `neo-server` 的可选特性 `jemalloc` 打开。这个特性只替换可执行文件的 Rust 全局分配器，SQLite 等 C 代码仍使用 libc 的 malloc，外部 ffmpeg 进程也不受影响。glibc 构建（Nix 包、Docker 镜像、NixOS 模块）不打开它。`MALLOC_ARENA_MAX` 只作用于 glibc，对静态包无效。

#### 分配器测量

测量在 24 核、约 92 GiB 内存的 x86_64 主机上进行，使用合成媒体和端到端测试用的假 OpenList。每个变体用同一份源码、`embed-web` 和 release profile 构建，串行运行：`soak.py` 以 6 个听众分别读取远端和本地媒体库，覆盖 Range 读取、拖动、换轨、响度和波形、强制转码、HLS 视频和第 30 分钟的远端重扫，负载一小时后再空闲十分钟，每 30 秒采样一次。之后用 `bench.py` 对 3000 个作品做全量扫描，再做 6 路并发播放、转码和重扫的组合负载。所有运行都没有请求错误。

| 变体 | 浸泡 RSS 峰值 / 空闲十分钟后 MiB | 浸泡 API p50 / p95 / p99 ms | 服务端 CPU s/h |
|---|---|---|---|
| glibc 默认 | 50.1 / 28.4 | 1.23 / 1.62 / 2.29 | 13.0 |
| glibc，`MALLOC_ARENA_MAX=2` | 42.4 / 21.1 | 1.22 / 1.45 / 1.57 | 12.4 |
| glibc + jemalloc 5.3.1 | 38.1 / 31.6 | 1.23 / 1.68 / 2.01 | 13.7 |
| glibc + mimalloc 2.3.2 | 122.9 / 64.1 | 1.20 / 1.43 / 1.67 | 13.6 |
| glibc + mimalloc 3.3.2 | 111.9 / 93.7 | 1.23 / 1.47 / 1.61 | 12.7 |
| musl mallocng | 18.6 / 16.8 | 1.28 / 1.53 / 1.66 | 13.9 |
| musl + jemalloc 5.3.1 | 35.0 / 25.3 | 1.28 / 1.55 / 1.70 | 13.1 |
| musl + mimalloc 2.3.2 | 116.0 / 98.5 | 1.27 / 1.55 / 1.69 | 13.3 |
| musl + mimalloc 3.3.2 | 120.2 / 87.1 | 1.25 / 1.65 / 1.79 | 13.1 |

浸泡负载很轻，各变体的延迟差别在噪声以内。默认 glibc 那一轮的主机负载平均值为 9，其余多在 1 到 3 之间，它的 p99 不能单独作为证据。重负载的差别来自 `bench.py`，下表的四个变体交错各跑四轮，取中位数：

| 变体 | 扫描 s（四轮范围） | 组合负载 API p50 / p95 ms | 服务端 RSS 峰值 MiB |
|---|---|---|---|
| glibc，`MALLOC_ARENA_MAX=2` | 4.0（3.8–9.2） | 4.9 / 6.6 | 141 |
| glibc + jemalloc | 4.0（3.8–9.4） | 4.2 / 5.7 | 151 |
| musl mallocng | 6.6（5.2–10.8） | 6.0 / 7.9 | 122 |
| musl + jemalloc | 4.5（4.4–4.6） | 5.05 / 6.45 | 144 |

结论如下：

- 静态包改用 jemalloc。mallocng 的内存最低，但在组合负载下 p50 和 p95 比 jemalloc 慢约 20%，四轮结果一致，扫描时间也波动更大。jemalloc 的代价是轻负载 RSS 多约 10 MiB，可执行文件大 0.6 MiB。
- glibc 构建保留系统 malloc 和 `MALLOC_ARENA_MAX=2`。jemalloc 只在组合负载下把 p50 降低约 14%，空闲后反而多留约 10 MiB。arena 上限让空闲后的 RSS 比默认少约 7 MiB。早先观察到的每小时约 12 MiB 增长在这次一小时浸泡中没有复现，两条 glibc 曲线都在下降，所以这组数据只说明上限没有可测的代价，不足以重新证明它是必需的。
- 不使用 mimalloc。两个版本的 RSS 都是其他方案的三到五倍，延迟没有更好。smaps 显示多出的部分几乎都是匿名透明大页：程序实际在用的堆只有 4 到 5 MB（glibc `mallinfo2` 和 jemalloc `stats.allocated` 的读数），mimalloc 保留的空闲页被内核合并成 2 MiB 大页，空闲期间 RSS 还会上升。设置 `MIMALLOC_ALLOW_THP=0` 和 `MIMALLOC_PURGE_DELAY=0` 的五分钟筛选把匿名内存降到约 11 MiB，但其中出现了换出，调参后的完整一小时运行没有做。

版本、统计接口和页大小等适用边界见[分配器版本与移植边界](allocator-sources.md)。jemalloc 在构建时确定页大小，将来发布 aarch64 静态包时需要在 16 KiB 和 64 KiB 页的内核上单独验证。

## 已知覆盖缺口

- 连续切换分页并反序返回响应、空库与越界页的生产包用例仍需补充，不能用现有深链接和普通翻页测试代替。
- 导入评价与播放历史的合并虽有结果断言，仍缺专门交换处理顺序的回归用例。
- 修改密码和删除用户的 API 用例尚需像退出所有设备一样，通过真实媒体路由断言旧媒体 token 被拒绝。
- PostgreSQL 只重跑检查矩阵中的选定目标，不能声称两个数据库运行了所有相同测试。
- WebKit 不在 Linux 的常规 Playwright 检查中，iOS 模拟器另行运行，真机尚需验证。
- HLS 主矩阵主要走 Chromium 原生实现，hls.js 只有基础播放覆盖，故障恢复矩阵尚不完整。
- 视频服务重启恢复由进程测试覆盖，没有相应浏览器重启用例。WebM 的 VP8、VP9 和 AV1 没有单独列入主视频矩阵。
- 导入封面保护、取消后的子进程与临时文件清理由后端覆盖。部分管理界面只有 mock 测试，不能据此声明真实后端流程已验证。
- Windows 的路径用例没有在 Windows 上运行整套 `cargo test`。交叉编译、安装包冒烟和 WebDriver 覆盖不同边界。
- 高负载下的测试超时、缓存预算边界与播放器恢复问题仍在[路线图](roadmap.md)中跟踪。
