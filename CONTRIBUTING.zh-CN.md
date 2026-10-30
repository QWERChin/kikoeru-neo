# 参与 kikoeru-neo 开发

构建 Rust 后端和 Svelte 前端，用合成数据运行它们，并在提交拉取请求前跑完检查。开发环境有三种选择：Dev Container 只需要 Docker，Nix 开发环境需要支持 flakes 的 Nix，也可以自己安装工具。三种方式提供同样的工具，准备好之后的命令都一样。如果只是安装软件，请从[使用指南](docs/zh-CN/index.md)开始。

## 准备开发环境

### Dev Container（Docker）

`.devcontainer/` 描述一个已经装好全部工具的 Debian 容器。镜像构建时由 Nix 准备好开发环境，容器里的每个 shell 启动时都会加载它，所以进去之后不用运行 `nix develop`，进入时也不会下载任何东西。需要 Linux 上的 Docker Engine 25 或更新版本，或者 Docker Desktop。第一次构建镜像要下载工具链、测试用的浏览器和 PostgreSQL，在一台 24 核的测试机上用了大约 12 分钟，镜像约占 11 GB 磁盘。只有 `flake.nix`、`flake.lock` 或开发环境用到的 Nix 文件改变时，镜像才需要重建。

- **VS Code**：安装 Dev Containers 扩展，打开检出目录，运行“Dev Containers: Reopen in Container”。
- **JetBrains IDE**：从检出目录打开 `.devcontainer/devcontainer.json`，选择创建 Dev Container。
- **不用 IDE**：在仓库根目录运行

  ```sh
  docker compose -f .devcontainer/compose.yml build
  docker compose -f .devcontainer/compose.yml run --rm --service-ports dev
  ```

检出目录挂载在 `/workspace`，容器以 uid 和 gid 1000 运行。宿主机用户的 uid 不是 1000 时，容器写出的文件在宿主机上属于 uid 1000。5173（Vite）和 8888（服务端）两个端口转发到宿主机的 `127.0.0.1`。在容器里启动 Vite 要用 `pnpm dev --host 0.0.0.0`，服务端要监听 `0.0.0.0:8888`，否则转发的端口连不到它们。

### Nix

在 `x86_64-linux` 或 `aarch64-linux` 上安装支持 flakes 的 Nix，然后在检出目录进入开发环境：

```sh
nix develop
```

### 自己安装工具

下表是 flake 固定、CI 测试的版本。其他较新的版本通常也能用，但没有经过测试。

| 工具 | 版本 | 用途 |
|---|---|---|
| Rust（rustc、cargo、rustfmt、clippy） | 1.98 | 后端。rust-analyzer 可选，供编辑器使用。 |
| C 工具链（gcc 或 clang）和 libc 头文件 | 当前版本即可 | Rust 依赖的构建脚本，包括随 SQLx 编译的 SQLite。 |
| mold | 2.42 | 必需：`.cargo/config.toml` 在 Linux 上用 `-fuse-ld=mold` 链接。 |
| Node.js | 22 | 前端。 |
| pnpm | 12.9 | 按 `web/pnpm-lock.yaml` 安装前端依赖。 |
| ffmpeg 和 ffprobe | 9.0，完整版 | 运行服务端，以及测试时生成合成媒体。测试需要 `libx264`、`aac`、`libmp3lame`、`libvpx-vp9`、`libopus`、`mpeg4`、`flac` 编码器和 lavfi 输入源，发行版的完整 `ffmpeg` 包都带有这些。 |
| sqlx-cli | 0.9，带 SQLite 和 PostgreSQL 支持 | 修改查询或迁移后运行 `scripts/sqlx-prepare.sh`。 |
| PostgreSQL | 18 | PostgreSQL 测试和 `sqlx-prepare.sh` 用到的 `initdb`、`pg_ctl`。以普通用户运行。 |
| Python 3，带 fonttools 和 brotli | 3.14，fonttools 4.63 | `nix/fallback-fonts.py`（生成字幕回退字体）和 `scripts/bench.py`。 |
| Liberation Sans 与 Noto Sans CJK 字体 | 2.1、2.004 | 嵌入前端的字幕回退字体的来源。 |
| Playwright 浏览器 | 与 `@playwright/test` 1.63.0 对应的 Chromium 和 Firefox | 浏览器测试。在 `web/` 运行 `pnpm exec playwright install --with-deps chromium firefox` 安装。中文界面还需要系统装有 CJK 字体，截图里才有文字。 |
| mdBook | 0.5 | 构建设计书。 |

Linux 桌面应用是 `desktop/` 下单独的 workspace，另外需要 pkg-config、GTK 3、WebKitGTK 4.1、libsoup 3、librsvg、libayatana-appindicator，以及带 base、good、bad、libav 插件的 GStreamer，都要带开发头文件。NixOS 虚拟机检查需要 Nix 和 KVM，没有它们就不能运行。

## 构建

保留主机的 Cargo 构建目录设置，不设置 `CARGO_TARGET_DIR`，也不向仓库的 `.cargo/config.toml` 添加 `target-dir` 或 `build-dir`。

`cargo build --locked -p neo-server` 构建只提供 API 的服务端，配合 Vite 开发服务器做后端开发已经够用。生产服务端要嵌入构建好的前端：

- 用 Nix 时，`nix build .#neo-server` 生成 `result/bin/kikoeru-neo`，嵌入前端，并提供精简版 ffmpeg。`nix build .#neo-server-static` 生成发布用的 `x86_64` 全静态二进制。
- 用 Docker 时，`docker build -t kikoeru-neo .` 构建发布镜像。

### 不用 Nix 构建完整服务端

前端构建会嵌入两个字幕回退字体及其许可证。在 Dev Container 和 Nix 开发环境里，`KIKOERU_FALLBACK_FONTS_DIR` 已经指向生成好的字体，构建命令是：

```sh
(cd web && pnpm install --frozen-lockfile && pnpm build)
mkdir -p web/build/licenses
cp "$KIKOERU_FALLBACK_FONTS_DIR/LICENSE-latin.txt" web/build/licenses/fallback-font-latin.txt
cp "$KIKOERU_FALLBACK_FONTS_DIR/LICENSE-cjk.txt" web/build/licenses/fallback-font-cjk.txt
cargo build --locked --release -p neo-server --features embed-web
```

自己安装工具时，要先生成字体。以 Debian 13 为例，先装发行版的软件包，再用 [rustup](https://rustup.rs) 安装 Rust，从 [nodejs.org](https://nodejs.org) 安装 Node.js 22（Debian 13 自带的是 Node.js 20），最后用 `npm install -g pnpm@12.9.0` 安装 pnpm：

```sh
sudo apt install build-essential mold git curl python3 python3-fonttools \
  python3-brotli fonts-liberation fonts-noto-cjk ffmpeg
```

然后在仓库根目录运行：

```sh
export KIKOERU_FALLBACK_FONTS_DIR="$PWD/dev-data/fonts"
mkdir -p "$KIKOERU_FALLBACK_FONTS_DIR"
python3 nix/fallback-fonts.py latin \
  /usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf \
  "$KIKOERU_FALLBACK_FONTS_DIR/fallback-latin.woff2"
python3 nix/fallback-fonts.py cjk \
  /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
  "$KIKOERU_FALLBACK_FONTS_DIR/fallback-cjk.woff2"
(cd web && pnpm install --frozen-lockfile && pnpm build)
mkdir -p web/build/licenses
cp /usr/share/doc/fonts-liberation/copyright web/build/licenses/fallback-font-latin.txt
cp /usr/share/doc/fonts-noto-cjk/copyright web/build/licenses/fallback-font-cjk.txt
cargo build --locked --release -p neo-server --features embed-web
```

其他发行版的字体路径不同，可以用 `fc-list | grep -E 'LiberationSans-Regular|NotoSansCJK-Regular'` 查找。`pnpm build` 同时生成 API 类型，这一步要用到 cargo。产物是 `target/release/kikoeru-neo`，一个内含前端的文件。它链接系统的 libc，和发布的二进制一样从 `PATH` 运行 ffmpeg 和 ffprobe。

## 运行后端与前端

在开发环境中创建独立的开发数据目录和配置。使用合成媒体，不连接已有安装的数据库或凭据：

```sh
umask 077
mkdir -p dev-data
printf 'listen = "127.0.0.1:8888"\n' > dev-data/config.toml
export KIKOERU_DATA_DIR="$PWD/dev-data"
export KIKOERU_CONFIG="$PWD/dev-data/config.toml"
unset KIKOERU_DATABASE_URL KIKOERU_LISTEN
cargo run --locked -p neo-server -- user create --admin admin
cargo run --locked -p neo-server -- serve
```

在 Dev Container 里改写成 `listen = "0.0.0.0:8888"`。创建账号时按提示输入至少 8 个字符的密码。后续运行复用这个目录和配置，只执行 `serve`。按 Ctrl-C 停止服务。

在同一个环境的第二个终端里（容器的另一个终端，或在仓库根目录再次进入 `nix develop`）启动前端：

```sh
cd web
pnpm install --frozen-lockfile
pnpm gen:types
pnpm dev
```

在 Dev Container 里给 `pnpm dev` 加上 `--host 0.0.0.0`。打开 Vite 输出的地址，通常为 `http://127.0.0.1:5173`。Vite 把 `/api` 转发到 `http://127.0.0.1:8888`，使用开发账号登录即可。不需要后端、只调试界面状态时，停止 Vite 后运行：

```sh
pnpm dev:mock
```

假数据模式使用 `contract/` 中的 JSON 样本，不能验证后端联调，也不能用于生产构建。界面目前使用简体中文。

## 运行检查

开发过程中用针对性检查，提交拉取请求前再完整跑一遍。下面的针对性检查在三种环境里都能运行。完整检查需要 Nix，Dev Container 里也有。从仓库根目录运行：

```sh
nix flake check
nix build .#book --no-link
```

`docker` 和 `nixos` 检查会启动 NixOS 虚拟机，需要 `/dev/kvm`，容器里通常没有。没有 Nix 时，跑完针对性检查后由 CI 在拉取请求上运行完整检查。`nix flake check` 构建当前系统定义的检查。以下检查只在 `x86_64-linux` 上定义：`rust-lint`、`rust-test`、`rust-test-pg`、`e2e`、`e2e-mock`、`windows`、`desktop`、`desktop-windows`、`docker` 和 `nixos`。`neo-typegen` 包的测试阶段也只在 x86_64 上运行。ARM64 检查通过不表示具备同等测试覆盖，GitHub CI 运行 x86_64 检查集合。

`rust-test` 在嵌入前端的条件下运行根 Rust workspace，桌面外壳属于单独的 workspace。`rust-test-pg` 只在 PostgreSQL 上运行选定的数据库、调度器和扫描测试，不是整个 workspace 的重跑。Windows 交叉检查只证明可编译，不证明运行行为。CI 的路径过滤会跳过纯文档改动，因此仍需在本地构建设计书。

后端的针对性检查：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked -p neo-core --test contract
cargo test --locked -p neo-server --test api_task_admin
cargo test --locked -p neo-scanner
scripts/sqlx-prepare.sh --check
```

将 crate 或测试目标换成改动影响的部分。前端检查从 `web/` 目录运行：

```sh
pnpm check
pnpm test
pnpm build
pnpm e2e:mock
```

针对生产服务端的浏览器测试需要嵌入了前端的服务端。用 `nix build .#neo-server` 或按[不用 Nix 构建完整服务端](#不用-nix-构建完整服务端)构建，然后在 `web/` 目录按对应路径运行：

```sh
KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e
KIKOERU_NEO_BIN=../target/release/kikoeru-neo pnpm e2e
```

Nix 开发环境和 Dev Container 都提供 Playwright 浏览器。针对性的 Cargo 和 pnpm 检查不能代替最终的 flake 检查。[测试与验证](book/src/testing.md)说明覆盖边界，以及自动化不能代替的设备验证。

## 保持代码与规格一致

[设计书](book/src/SUMMARY.md)是实现规格，只维护中文版。修改前先读概述和对应领域的章节。路线图按领域列出未完成的工作，设计决定页记录既有行为与开放问题。改变行为或设计决定时，在同一改动中更新相关章节。

- 接口以[合同章节](book/src/contract.md)和 `contract/` 中的 JSON 样本为准。改变线上格式时，先单独提交只含合同章节和样本的改动，标题为 `docs: update API contract for ...`。后续提交先更新后端 DTO 和合同测试，再更新前端。这些提交可以放在同一个拉取请求里。
- Rust 的 `neo-core::dto` 通过 ts-rs 和 `neo-typegen` 生成 TypeScript。在 `web/` 运行 `pnpm gen:types` 可以重新生成。不要手写 API 响应类型，也不要提交 `web/src/lib/api/types/` 中的生成文件。
- 共享业务 SQL 放在 `crates/neo-db/queries/`，分别对 SQLite 和 PostgreSQL 编译。迁移、数据库复制、后端专用语句和测试是例外，边界见[数据模型](book/src/data-model.md)。处理器、扫描和导入调用 `neo_db` 函数，不自行添加 SQL。
- 修改查询或迁移后，在开发环境中运行 `scripts/sqlx-prepare.sh`，把两个后端 crate 的 `.sqlx/` 一起提交。脚本使用临时数据库，结束时清理。
- 在行为变化的边界添加测试。测试和截图使用合成作品、媒体和凭据，不提交真实媒体库的标题、路径、封面、数据库内容或配置。

## 提交与拉取请求

每个提交只做一项逻辑改动。标题最多 72 个字符，格式为 `<prefix>: <imperative description>`，前缀使用 `feat`、`fix`、`refactor`、`docs`、`test`、`chore`、`perf` 或 `ci`。仅在需要解释原因或非显然约束时添加正文条目。最后使用贡献者自己的姓名和邮箱签署，不使用维护者身份：

```text
fix: preserve progress when an older report arrives

- Ignore stale reports so concurrent devices cannot overwrite newer progress.

Signed-off-by: Your Name <you@example.com>
```

向 `main` 提交拉取请求，说明改了什么、为什么改、如何验证，并写明跳过的检查和已知缺口。用户文档在 `docs/en/` 和 `docs/zh-CN/` 中平行维护，页内导航保持同一种语言，两套内容要一致。设计书使用简体中文，代码注释使用英文。

CI 运行时，必须全部通过才能合并，不能绕过失败项。维护者也可能运行 AI 辅助的正确性审查。逐项核实具体触发条件，为确认的缺陷补充修复和回归覆盖，并解释不采纳的结论。贡献者不需要 AI 订阅或维护者的审查环境，合并和发布由维护者执行。
