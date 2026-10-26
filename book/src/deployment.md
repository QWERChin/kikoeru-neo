# 构建与部署

本章面向实现者，说明开发环境、构建产物、打包约定与发布机制。`nix build` 构建的是嵌入了前端的 `neo-server` 生产包。

安装和日常运行见[中文使用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/index.md)，其中分别提供[NixOS 安装](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/install-nixos.md)、[Docker 安装](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/install-docker.md)和[升级](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/upgrade.md)步骤。

## 仓库结构

```text
kikoeru-neo/
  flake.nix            构建入口，包括后端、前端、桌面版、设计书、开发环境和 NixOS 模块
  nix/                 NixOS 模块、启动它的虚拟机测试、Docker 镜像和镜像的检查
  Cargo.toml           Rust workspace
  crates/
    neo-core/          模型类型、DTO、数据库的行类型、迁移
    neo-db/            存储层：共享业务查询（queries/），按配置选 SQLite 或 PostgreSQL
    neo-db-sqlite/     把 neo-db/queries 编译成 SQLite 版本，连接池
    neo-db-postgres/   把 neo-db/queries 编译成 PostgreSQL 版本，连接池
    neo-storage/       local、openlist、webdav 的适配层
    webdav-client/     只读的 WebDAV 客户端
    neo-scanner/       扫描和元数据
    neo-media/         ffmpeg 调度、缓存
    neo-server/        HTTP 服务和命令行（serve、user、import-legacy、backup）
    neo-typegen/       导出 DTO 的 TypeScript 类型
  web/                 SvelteKit 前端
  desktop/             Tauri 桌面版，单独的 Cargo workspace
  contract/            接口合同的 JSON 样本
  scripts/             基准、浸泡测试、sqlx 元数据、Docker 迁移、发布和 iOS 模拟器测试用的脚本
  book/                这本设计书
```

OpenList 的 API 客户端和限流在单独的仓库 `chinrw/openlist-client` 里，根目录的 `Cargo.toml` 按 `rev` 固定它，改它要先改那个仓库，再移动这里的 `rev`。

## 开发环境

常用的命令：

```sh
nix develop
cargo test --workspace --locked
cargo run --locked -p neo-typegen
nix build .#neo-typegen
nix build .#neo-server
cargo run --locked -p neo-server -- user create --admin admin --data-dir ./dev-data
cargo run --locked -p neo-server -- serve --data-dir ./dev-data
nix flake check
cd web && pnpm install --frozen-lockfile && pnpm check && pnpm test && pnpm build
```

开发环境提供固定在 `flake.lock` 中的 Rust 工具链、mdBook、Node 22、pnpm 和验证导出类型用的 TypeScript。`neo-typegen` 从仓库根目录运行时，把类型写到 `web/src/lib/api/types/`，每个类型一个文件，再写一个重新导出全部类型的 `index.ts`。前端的 `pnpm check`、`pnpm test` 和 `pnpm build` 会先通过 `pnpm gen:types` 重新生成这些文件。也可以传入输出目录，例如 `cargo run --locked -p neo-typegen -- /tmp/neo-types`。导出文件由构建生成，不手工维护，也不提交。

`.cargo/config.toml` 让 Linux 上直接运行的 cargo 用 mold 链接，开发环境提供这个工具。Nix 包用 crane 构建，源码里不带 `.cargo/`，所以用默认的链接器，不需要 mold。release 用完整的 LTO（`lto = "fat"`），`codegen-units` 为 1。这样用较长的链接时间换取较小的产物，发布包还去掉符号表（`strip = "symbols"`）。panic 时日志里的出错消息和代码位置（文件和行号）不靠符号表，照样有。去掉之后 `RUST_BACKTRACE=1` 打出的调用链和 perf、gdb 只剩地址，要查这类问题时，在本地用同一个提交构建一份不去符号的二进制。开发构建保留默认的增量编译和代码生成单元数量。在 Nix 开发环境外运行 cargo 时，也需要让 `mold` 在 PATH 中可用。这包括在 `desktop/src-tauri` 里直接构建桌面版，因为 cargo 会读到仓库根目录的 `.cargo/config.toml`。

开发环境里还有 sqlx-cli 和 PostgreSQL。共享业务查询写在 `crates/neo-db/queries/` 里，迁移、复制和后端专用语句的例外见[数据模型](data-model.md#存储层的代码结构)。共享查询被编译两次，所以有两份离线元数据：`crates/neo-db-sqlite/.sqlx/` 和 `crates/neo-db-postgres/.sqlx/`。`query!` 宏在没有设置 `DATABASE_URL`（SQLite）和 `PG_DATABASE_URL`（PostgreSQL）时读它们，平时开发不需要数据库。改了查询或者迁移之后，重新生成这两份并一起提交：

```sh
scripts/sqlx-prepare.sh
```

脚本在一个临时目录里建一个 SQLite 文件、起一个只监听 Unix socket 的 PostgreSQL，各自跑完迁移，再分别在两个 crate 里运行 `cargo sqlx prepare`，结束时删掉临时目录。两份元数据不能合成仓库根目录下的一份：同一条 SQL 的文件名相同，一个后端的会覆盖另一个的。

`nix flake check` 用 `cargo sqlx prepare --check` 确认两份都没有过期，`scripts/sqlx-prepare.sh --check` 在本地做同样的检查。Nix 的 `neo-server` 和 `neo-server-static` 发布包把提交号和提交时间写进 `/api/version`。`build_time` 用的是提交时间而不是实际构建时间，避免实际构建时钟改变包内容。直接用 cargo 或没有 Git 元数据的源码目录构建时这两项为 `null`。检查和端到端测试用的构建不带提交号，所以只改设计书的提交不会让它们重新构建。

Rust 构建用 crane。第三方依赖单独构建。在工具链、构建参数、锁文件和相关输入均相同时可复用依赖产物，不能仅凭 `Cargo.lock` 未变保证缓存命中。Clippy、测试和 neo-typegen 用 dev profile，省掉 release profile 的 LTO，只有发布包用 release。

开发环境里有 ffmpeg 和 ffprobe，neo-media 的测试直接运行它们。前端和设计书的本地预览：

```sh
cd web && pnpm dev   # 前端开发服务器，把 /api 代理到后端
mdbook serve book    # 本地预览设计书
```

### Docker 开发环境

`.devcontainer/` 为使用 Docker 的贡献者提供同一个开发环境。VS Code 可以重新在容器中打开仓库，JetBrains 可以使用仓库中的 Dev Container 配置。不使用 IDE 时，从仓库根目录运行：

```sh
docker compose -f .devcontainer/compose.yml build
docker compose -f .devcontainer/compose.yml run --rm --service-ports dev
```

容器使用 Debian 的 glibc 和 FHS 路径承载 IDE 远端服务，以 uid/gid 1000 写入绑定的源码目录。宿主用户不是 1000 时，须先协调绑定目录的权限，配置不自动改宿主文件的所有者。前端需运行 `pnpm dev --host 0.0.0.0`，后端需监听 `0.0.0.0:8888`，这样才能通过转发的 5173 和 8888 端口访问。

镜像构建时运行一次 `nix develop`，随后用 `nix print-dev-env --json` 导出环境。导出器去掉临时构建目录和 HOME 等进程身份变量，保留编译器参数、字体和浏览器路径。登录与非登录 Bash、`sh`、容器入口以及绕过入口的 `docker exec` 工具调用都加载预先导出的环境，不在进入时调用 Nix 或下载开发工具。开发环境通过 Nix profile 保留，后续垃圾回收不会删掉其闭包。

复制 Nix 时就把 `/nix` 的所有者设为 uid/gid 1000，开发闭包也由 `dev` 用户构建。闭包生成后不再递归修改 store 的所有者，避免 Docker 把整份闭包复制到新的文件层。

昂贵的环境层只复制 `flake.nix`、`flake.lock`、`nix/dev-shell.nix` 和回退字体的 Nix、Python 文件。应用源码绑定到 `/workspace`，不参与这一层的缓存键。更改开发环境后需要重新构建镜像。容器不设置 Cargo 构建目录，仓库 `.cargo/config.toml` 中的 mold 链接参数仍生效。首次前端依赖安装仍运行 `pnpm install --frozen-lockfile`，这不是进入容器时的自动步骤。

## 构建

`nix build` 按固定顺序构建：

1. `neo-typegen` 导出 DTO 类型到 `web/src/lib/api/types/`。
2. 构建前端（`.#web`）。依赖由 `fetchPnpmDeps` 按 `web/pnpm-lock.yaml` 预先下载，哈希固定在 `flake.nix` 里，构建本身不联网。检查阶段运行 `svelte-check` 和 Vitest，并确认产物里没有假数据。
3. 构建 `neo-server`，打开 `embed-web` 特性，把前端产物嵌进二进制。SQLx 用两个后端 crate 里的 `.sqlx/` 离线元数据，构建时不需要数据库。

改了 `web/package.json` 或 `web/pnpm-lock.yaml` 之后，要更新 `flake.nix` 里 `pnpmDeps` 的哈希：先把它改成 `pkgs.lib.fakeHash`，运行 `nix build .#web`，再把报错里 `got:` 后面的值填回去。

产出的包是后端二进制加一个包装脚本，包装脚本把 ffmpeg 和 ffprobe 放进 PATH，所以部署的机器上不需要另外装 ffmpeg。

服务端使用 `nix/ffmpeg.nix` 裁剪的 `ffmpeg-headless`，关闭不需要的外部编码库、网络功能和其他依赖，保留 H.264、AAC 转码以及响度和波形分析所需的组件，包括 PCM 输出。`ffmpeg` 检查只确认这些组件存在，不保证其他组件都已去掉。媒体输入仍受 `neo-media` 的格式白名单限制，不能由此推断任意媒体都可播放。

裁剪后的包需要独立构建，能否从缓存取得取决于固定输入与缓存配置。`ffmpeg` 检查验证实际需要的组件，解封装器清单读取 `crates/neo-media/src/input.rs`。测试与开发环境使用完整 ffmpeg，因为生成合成媒体需要额外编码器。

生产包用 `nix build` 构建，首次运行步骤见[快速开始](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/quickstart.md)。

设计书用 `nix build .#book` 单独构建。

### Linux 单文件服务端

`nix build .#neo-server-static` 在 x86_64 Linux 上用 `pkgsStatic` 构建 musl ELF，启用 `embed-web`，不带启动包装脚本、动态加载器或共享库依赖。SQLite 随 SQLx 编译，PostgreSQL 支持保持启用。产物是 `result/bin/kikoeru-neo`。

这个包不包含 ffmpeg 和 ffprobe。运行时仍按 `media.ffmpeg`、`media.ffprobe` 的配置或 PATH 查找它们，与普通 Cargo 构建相同。媒体能力的最低要求由 `nix/ffmpeg.nix` 的检查列出，只有二进制能够启动不代表转码已经可用。

CLI 的 `serve` 每次启动时在后台检查一次配置的工具，不等待检查完成，也不把结果纳入 `/api/ready`。其他子命令和桌面嵌入服务不运行这项检查。每条命令最多运行 5 秒，超时后终止子进程。检查先运行两个工具的 `-version`，再读取 ffmpeg 的编码器、封装器和滤镜列表，确认 `libx264`、`aac`、`pcm_s16le`，`hls`、`mpegts`、`mp4`、`s16le`，以及 `scale`、`asplit`、`ebur128`、`anullsink`、`aresample`、`aformat` 存在。

工具不可执行、查询失败、超时或缺少组件时，每个问题记录一条 WARN，包含尝试的可执行文件路径、受影响功能和 `media.ffmpeg` 或 `media.ffprobe` 配置键。默认裸文件名表示按 PATH 查找。检查失败不阻止启动，浏览器原生支持的格式仍可播放。组件列表只能提前发现缺失，不能证明所有输入都能解码，也不能代替实际转码测试。发行版的精简包可能缺少 `libx264`，需要安装完整构建或配置另一份 ffmpeg。

静态包保留锁定的 musl 1.2.6 默认 mallocng 分配器，不另加全局分配器。`MALLOC_ARENA_MAX` 限制的是 glibc arena，对这个包无效，不能照搬 Docker 的环境变量当作内存优化。当前没有等负载的测量证明更换分配器有收益，也不据启动检查推断长期 RSS。远端并发转发的浸泡比较仍需使用合成数据，见[测试与验证](testing.md#常驻内存的增长)。

### 嵌入前端

前端产物用 rust-embed 嵌进二进制，由 `neo-server` 的 cargo 特性 `embed-web` 控制，默认关闭。只改后端时，`cargo build` 和 `cargo test` 不需要先构建前端，这时除了 API 以外的路径都返回 JSON 404。要在本地得到和生产包一样的服务：

```sh
cd web && pnpm build && cd ..
cargo run --release -p neo-server --features embed-web -- serve --data-dir ./dev-data
```

release 构建把 `web/build` 编进二进制。debug 构建在运行时从 `web/build` 读文件，所以重新 `pnpm build` 之后不用重新编译后端。

服务端怎么处理根路径下的请求，见[前端](frontend.md)的“发布方式”。

## 部署目标

NixOS 模块管理服务账号、只读配置、数据目录和 systemd 单元，不管理 DNS、证书、反向代理或防火墙。命令包装器让服务和管理命令默认选中同一配置，避免 PostgreSQL 安装的管理命令误写 SQLite。

单元以非特权用户运行，只让数据目录可写。不开启 `RestrictSUIDSGID=`，因为 systemd 的 seccomp 过滤看不到 `openat2` 结构体中的标志，会让它返回 `ENOSYS`。存储层仍在系统调用不可用时提供逐段 `O_NOFOLLOW` 的回退。新增单元限制后，应使用合成本地媒体验证实际文件访问。

迁移失败会退出，systemd 每 5 秒重试，5 分钟内失败 5 次后保持失败状态。不能因重启策略掩盖迁移错误，也不能把服务可就绪当成管理员、媒体库或外部连接已配置成功。

`nix/test.nix` 在虚拟机中检查模块启动、权限、账号、运行时备份和恢复，并用另一节点验证本机 PostgreSQL 与管理命令的目标一致性。平台范围见[测试与验证](testing.md)。

普通 Linux 的 systemd 单元保存在 `packaging/systemd/kikoeru-neo.service`，运行 `/usr/local/bin/kikoeru-neo` 并读取 `/etc/kikoeru-neo/config.toml`。安装者创建专用系统用户，systemd 用 `StateDirectory` 创建属于它的 `/var/lib/kikoeru-neo`，权限为 `0700`。单元沿用 NixOS 模块的重启策略和适用的沙箱限制，保留 systemd 默认 PATH，因此可以找到 `/usr/bin/ffmpeg` 和 `/usr/bin/ffprobe`。

首次启动后须等待 `/api/ready` 成功，再运行管理命令，避免与建库迁移并发。管理命令以同一用户运行，通过 `env -i` 显式提供 `KIKOERU_DATA_DIR`、`KIKOERU_CONFIG` 和系统 PATH，并在 shell 中设置 `umask 077`，避免继承管理员的数据库覆盖变量。备份目标放在数据目录之外，由安装者创建私有父目录。`nix/systemd-test.nix` 安装仓库中的原始单元和静态包，验证这套命令、权限、真实响度分析、停服备份和缺少 ffmpeg 时的告警，不代替各发行版的安装验证。

## 安全

网络入口、HTTP 限制和可信代理的操作约定见[安全与网络](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/security.md)。下面保留实现层的防护机制，认证细节见[认证与用户](auth.md)。

### 已有的防护

- **token。** API token 只能放在 `Authorization` 头里，媒体 token 只能放在 `?token=` 里（封面另外接受 cookie `kikoeru-media` 和 HTTPS 下的 `__Secure-kikoeru-media`，地址里平时因此不带 token），并且只在只读的媒体接口上有效，规则见[认证](auth.md)。每个请求都重新读用户行并比较 `token_version`，删除用户、改密码马上生效。日志里的 URL 去掉了 `token` 参数。
- **登录。** 每个来源在当前和上一分钟窗口累计达到 10 次失败后暂时拒绝，按用户名的失败只延迟不拒绝，所以别人猜错一个用户名的密码不会把那个用户锁在外面。
- **CSP。** 页面的策略由前端构建写进页面的 `<meta http-equiv>`（`web/vite.config.ts` 里 SvelteKit 的 `csp`，哈希模式）：脚本只能来自本服务和 SvelteKit 启动脚本的哈希，另外允许 `'wasm-unsafe-eval'` 给 JASSUB 编译 WebAssembly，`blob:` 给 hls.js 的 worker 和 MSE 播放，`ipc:` 和 `http://ipc.localhost` 给桌面版外壳的 IPC。样式保留 `'unsafe-inline'`，因为 Svelte 和 Vidstack 都写内联的 `style` 属性。页面和静态文件的响应头只加 `frame-ancestors 'none'`，因为 meta 里写不了它，响应头里更严的策略还会作用到 JASSUB 的 worker 上。API 的回答和媒体库里的文件带 `default-src 'none'; frame-ancestors 'none'`，所以单独打开媒体库里的一个文件也不会执行或加载任何东西。PDF 例外，只带 `frame-ancestors 'none'`，因为浏览器的 PDF 阅读器算插件，会被 `default-src 'none'` 挡住。
- **响应头。** 所有响应都带 `X-Frame-Options: DENY` 和 CSP 的 `frame-ancestors 'none'`，局域网里别的网页不能把应用嵌进去诱导管理员点击。`Referrer-Policy: no-referrer` 让带媒体 token 的地址不会出现在 Referer 里。没有自己缓存规则的 API 回答都带 `Cache-Control: no-store`，其中包括登录和换媒体 token 的回答。
- **路径。** 本地媒体库用 `openat2` 的 `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS` 打开文件，`openat2` 不可用时逐段带 `O_NOFOLLOW` 打开，符号链接都不跟随。OpenList 和 WebDAV 返回的名字里有 `..`、`/`、`\` 或 NUL 时拒绝。缓存文件名是摘要，HLS 分片只接受 `seg-NNNNN.ts`，字体和字幕按数据库 ID 取，打包下载的条目名去掉了 `..` 和控制字符。
- **ffmpeg。** 只用参数数组启动，不经过 shell。输入总是 `-protocol_whitelist file` 加单文件格式的白名单，远端文件先复制到 `cache/tmp/` 再交给 ffmpeg，所以文件名和内容都不能让它去读别的文件或网址。
- **远端文件。** 浏览器拿不到 OpenList 的直链，文件经本服务转发，`Content-Type` 由本服务决定。客户端 60 秒不读数据就断开转发，放出这个媒体库的传输名额，所以暂停的播放器不会占住名额。
- **OpenList 给的直链。** 直链来自 OpenList 的回答，openlist-client 从 v0.4.0 起不连本机回环、链路本地和 `0.0.0.0/8` 的地址，重定向的每一跳和解析出的地址都检查。OpenList 自己的地址和端口除外，因为本机存储的直链指回 OpenList。这样即使 OpenList 被人控制，也不能借本服务读这台机器上别的服务或者云主机的元数据接口。

### 已知边界

管理员能够选择后端可读目录和内网地址，属于可信主体。应用没有按用户隔离媒体库的权限，也没有完整的入口请求超时和连接数限制。OpenList 直链仍允许局域网目标，元数据来源响应没有统一大小上限，这些都需要部署层的权限和网络边界约束。

依赖通告和不适用项的理由保存在 `.cargo/audit.toml`。审计检查覆盖服务端和桌面版的锁文件，以及前端全部依赖。前端运行时依赖也在 `devDependencies` 中，不能用 `pnpm audit --prod` 跳过它们。安全边界的变更需要对应的回归测试，不能从一次部署的信任假设推导一般保证。

## Docker

只安装 Docker 的机器可以在仓库根目录运行 `docker build -t kikoeru-neo .`。根目录 Dockerfile 在按标签和多架构摘要固定的 Nix builder 中构建 `docker-rootfs`，最终 `FROM scratch` 只复制运行闭包。x86_64 和 aarch64 都使用原生 builder，不做交叉模拟。`.dockerignore` 只允许构建输入进入上下文，不带 `.git` 或本地数据，因此该路径不写提交号和提交时间，也不提供提交号 build argument。

原有 `nix/docker.nix` 仍用 `streamLayeredImage` 生成镜像流，服务层与依赖层分开。两条路径共用 `nix/docker-runtime.nix` 中的文件、权限命令和配置。`docker-rootfs` 用带数字所有者的 tar 保存运行闭包，因为 Nix store 目录本身不能保留 uid 1000 的所有权。Dockerfile 的静态元数据由 `dockerfile` 检查与共享配置逐项比较，修改版本号时也须同步版本 label。Dockerfile 的层布局不承诺与 Nix 镜像相同。构建、载入和 Compose 步骤见[Docker 安装](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/install-docker.md)。

### 镜像的约定

- 进程以 uid 和 gid 1000 运行，入口由 tini 转发信号并回收子进程，再通过 `umask 077` 包装器启动服务。默认参数选择显式配置文件，缺少配置时失败。
- `/data` 与 `/backup` 是空的私有目录，首次挂载命名卷时继承所有权和权限。保持 `/data` 为空是空目标导入的前提，备份则必须写到数据目录外。
- SQLite 临时文件优先放在数据卷缓存中，避免大型索引或备份占满内存中的 `/tmp`。目录尚不存在时 SQLite 可以回退到 `/tmp`。
- 健康检查从 `KIKOERU_LISTEN` 解析地址，通过 bash 的 TCP 重定向访问 `/api/ready`，不依赖 curl 或 wget。镜像没有 `/bin/sh`，不能用隐式 shell 的健康检查形式。
- 远端媒体客户端的证书根随二进制编译，挂载系统 CA 文件不会自动改变它。PostgreSQL 的 TLS 配置是独立路径。

### 从旧 kikoeru 容器迁移

迁移脚本只通过检查和复制读取停止的源容器。它将源配置、数据库和封面复制到私有暂存目录，再以只读挂载交给新镜像。新卷使用本次运行的标签确认所有权，避免清理另一个进程创建的同名卷。目标挂载必须避开继承的旧数据挂载，配置输出也不能覆盖已有文件。

虚拟机检查使用合成的、从未启动的旧容器，覆盖 bind mount、容器可写层、自定义数据路径、单文件挂载、共享数据库与封面目录和取消清理。脚本级测试用假 Docker 拒绝重复挂载，并检查从不启动源容器。用户步骤见[迁移指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/migration.md)。

### 发布

发布工作流的目标是 `ghcr.io/qwerchin/kikoeru-neo`，目前只发布 x86_64 镜像。镜像的 `org.opencontainers.image.source` 指向仓库，`org.opencontainers.image.licenses` 为 `GPL-3.0-or-later`。source label 让包关联仓库并继承访问权限，但不继承 public 可见性。GHCR 首次创建包仍默认 private，维护者须在包设置中改为 public，并验证匿名拉取，规则见 [GitHub 文档](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)。

- **标签。** `X.Y.Z` 固定一个版本，`X.Y` 指向该系列最新补丁，不发布 `latest`。需要可重复部署时固定完整版本或镜像摘要。
- **触发。** `.github/workflows/release.yml` 响应 `vX.Y.Z` 标签，并允许手动重试已有标签。版本必须与 `Cargo.toml` 一致。工作流运行检查、构建镜像，再由 `scripts/ghcr-release.sh` 推送。
- **重复发布。** `X.Y.Z` 已存在且镜像 config 摘要相同时，脚本跳过它，只补浮动标签。摘要不同或查询失败时拒绝推送。`X.Y` 只向较新的补丁版本移动，不因重试旧版本回退。
- **并发。** 发布共用一个并发组，避免两个运行同时移动浮动标签。被排队规则取消或推送失败的运行需要重新触发，不能把触发成功当成发布完成。
- **验证。** `ghcr-release` 检查使用假 skopeo，验证流程而不推送镜像。删除旧发布前应考虑仍固定这些版本的消费者，不复用已经发布过的版本号。

同一标签还创建 GitHub Release，上传 `kikoeru-neo-X.Y.Z-x86_64-linux-musl` 和 `SHA256SUMS`。检查任务只有读取权限，镜像任务另授 `packages: write`，二进制任务另授 `contents: write`。所有外部 Actions 固定完整提交 SHA。

`scripts/github-release.py` 要求标签已经存在且与 Cargo 版本一致。重试时复用已有 Release，下载同名资产比较实际 SHA-256，内容相同则保留，内容不同或 API 查询失败则停止，不自动替换。新 Release 先建为 draft，两个资产上传完后才发布。中断后可以补齐缺少的资产，再发布同一个 draft。`github-release` 检查通过假 API 验证这些分支，不接触 GitHub。

### 镜像的检查

`nix flake check` 里的 `docker-image` 检查不需要 Docker：它把镜像流写成 tar，检查用户、入口、默认参数、环境变量、卷、端口和健康检查的配置，检查 `/data` 是空的、`0700`、属于 1000，`/backup` 存在，`/tmp` 是 `1777`，neo-server 单独占一层，并用 `bash -n` 检查健康检查脚本。它把 neo-server 这一层和整个镜像的大小写进 `result/sizes.txt`。

`docker-rootfs` 检查独立检查归档中的卷权限和可执行文件链接。`dockerfile` 检查比较 ENTRYPOINT、CMD、USER、ENV、EXPOSE、VOLUME、完整 HEALTHCHECK 和 LABEL，拒绝未覆盖的最终阶段指令。GitHub CI 另外在 Ubuntu 上原生构建两种 Dockerfile，并运行真实 Docker 冒烟测试。只有容器、Nix、Cargo、前端、合同和相关 CI 脚本等构建输入变化时才运行这两个昂贵任务，手动触发则总是运行。结构检查不能替代真实 Docker 的 COPY、卷初始化或 IDE 进入验证。

## 升级与版本恢复

软件版本、数据库迁移和外部状态必须匹配。服务恢复调度时把运行中任务标为中断，并恢复排队任务，不能通过重启承诺扫描自动续跑。数据库版本回退也不能只换程序，需要匹配的状态恢复点。版本固定、升级核对和恢复流程见[升级指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/upgrade.md)及[备份与恢复](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/backup-restore.md)。
