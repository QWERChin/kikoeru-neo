# 参与 kikoeru-neo 开发

使用 Nix 开发环境构建 Rust 后端和 Svelte 前端，并用合成数据验证改动。需要 Git，以及在 `x86_64-linux` 或 `aarch64-linux` 上启用了 flakes 的 Nix。如果只是安装软件，请从[使用指南](docs/zh-CN/index.md)开始。

## 准备环境与构建

在仓库目录运行：

```sh
nix develop
cargo build --locked -p neo-server
nix build .#neo-server
```

开发环境提供 Rust、mold、Node、pnpm、ffmpeg、ffprobe、mdBook、SQLx CLI 和 PostgreSQL。Cargo 构建只提供 API，不带前端。Nix 包位于 `result/bin/kikoeru-neo`，嵌入生产前端，并提供 ffmpeg 和 ffprobe。

保留主机的 Cargo 构建目录设置，不设置 `CARGO_TARGET_DIR`，也不向仓库的 `.cargo/config.toml` 添加 `target-dir` 或 `build-dir`。

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

创建账号时按提示输入至少 8 个字符的密码。后续运行复用这个目录和配置，只执行 `serve`。按 Ctrl-C 停止服务。

在第二个终端中，从仓库根目录进入 `nix develop`，再启动前端：

```sh
cd web
pnpm install --frozen-lockfile
pnpm gen:types
pnpm dev
```

打开 Vite 输出的地址，通常为 `http://127.0.0.1:5173`。Vite 把 `/api` 转发到 `http://127.0.0.1:8888`，使用开发账号登录即可。不需要后端、只调试界面状态时，停止 Vite 后运行：

```sh
pnpm dev:mock
```

假数据模式使用 `contract/` 中的 JSON 样本，不能验证后端联调，也不能用于生产构建。界面目前使用简体中文。

## 运行检查

从仓库根目录运行：

```sh
nix flake check
nix build .#book --no-link
```

`nix flake check` 构建当前系统定义的检查。以下检查只在 `x86_64-linux` 上定义：`rust-lint`、`rust-test`、`rust-test-pg`、`e2e`、`e2e-mock`、`windows`、`desktop`、`desktop-windows`、`docker` 和 `nixos`。`neo-typegen` 包的测试阶段也只在 x86_64 上运行。ARM64 检查通过不表示具备同等测试覆盖，GitHub CI 运行 x86_64 检查集合。

`rust-test` 在嵌入前端的条件下运行根 Rust workspace，桌面外壳属于单独的 workspace。`rust-test-pg` 只在 PostgreSQL 上运行选定的数据库和扫描测试，不是整个 workspace 的重跑。Windows 交叉检查只证明可编译，不证明运行行为。CI 的路径过滤会跳过纯文档改动，因此仍需在本地构建设计书。

开发过程中，可以在 `nix develop` 中执行针对性检查：

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

生产包浏览器测试需要先在根目录运行 `nix build .#neo-server`，然后在开发环境的 `web/` 目录执行：

```sh
KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e
```

开发环境提供 Playwright 浏览器。针对性的 Cargo 和 pnpm 检查不能代替最终的 flake 检查。[测试与验证](book/src/testing.md)说明覆盖边界，以及自动化不能代替的设备验证。

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
