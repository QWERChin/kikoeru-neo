# Contributing to kikoeru-neo

Build the Rust backend and the Svelte frontend, run them against synthetic data and run the checks before you open a pull request. Choose one of three development environments: the Dev Container needs only Docker, the Nix shell needs Nix with flakes, and you can also install the tools yourself. All three provide the same tools, and the commands after setup are the same. For installation rather than development, start with the [user guide](docs/en/index.md).

## Set up a development environment

### Dev Container (Docker)

The `.devcontainer/` directory describes a Debian container with every tool already installed. Nix prepares the environment while the image builds, and each shell in the container loads it on start, so you never run `nix develop` inside and nothing is downloaded when you enter. You need Docker Engine 25 or newer on Linux, or Docker Desktop. The first image build downloads the toolchain, test browsers and PostgreSQL, which took about 12 minutes on a 24-core test machine. The image needs about 11 GB of disk. The image is rebuilt only when `flake.nix`, `flake.lock` or the Nix files of the development shell change.

- **VS Code**: install the Dev Containers extension, open the checkout and run “Dev Containers: Reopen in Container”.
- **JetBrains IDEs**: open `.devcontainer/devcontainer.json` from the checkout and choose to create a Dev Container.
- **Without an IDE**: from the repository root, run

  ```sh
  docker compose -f .devcontainer/compose.yml build
  docker compose -f .devcontainer/compose.yml run --rm --service-ports dev
  ```

The checkout is mounted at `/workspace` and the container runs as uid and gid 1000. If your host user has another uid, files the container writes belong to uid 1000 on the host. Ports 5173 (Vite) and 8888 (server) are forwarded to `127.0.0.1` on the host. Inside the container, start Vite with `pnpm dev --host 0.0.0.0` and let the server listen on `0.0.0.0:8888`, or the forwarded ports cannot reach them.

### Nix

Install Nix with flakes enabled on `x86_64-linux` or `aarch64-linux`, then enter the shell from the checkout:

```sh
nix develop
```

### Install the tools yourself

These are the versions the flake pins and CI tests. Other recent versions usually work but are not tested.

| Tool | Version | Used for |
|---|---|---|
| Rust (rustc, cargo, rustfmt, clippy) | 1.98 | The backend. rust-analyzer is optional, for editors. |
| A C toolchain (gcc or clang) and libc headers | Any current | Build scripts of Rust dependencies, including the bundled SQLite. |
| mold | 2.42 | Required: `.cargo/config.toml` links with `-fuse-ld=mold` on Linux. |
| Node.js | 22 | The frontend. |
| pnpm | 12.9 | Frontend dependencies from `web/pnpm-lock.yaml`. |
| ffmpeg and ffprobe | 9.0, a full build | Running the server and generating synthetic media in tests. Tests need the encoders `libx264`, `aac`, `libmp3lame`, `libvpx-vp9`, `libopus`, `mpeg4`, `flac` and the lavfi sources, which a distribution's full `ffmpeg` package has. |
| sqlx-cli | 0.9, with SQLite and PostgreSQL | `scripts/sqlx-prepare.sh` after query or migration changes. |
| PostgreSQL | 18 | `initdb` and `pg_ctl` for the PostgreSQL tests and `sqlx-prepare.sh`. Run them as a normal user. |
| Python 3 with fonttools and brotli | 3.14, fonttools 4.63 | `nix/fallback-fonts.py`, which builds the subtitle fallback fonts, and `scripts/bench.py`. |
| Liberation Sans and Noto Sans CJK fonts | 2.1, 2.004 | Sources of the subtitle fallback fonts embedded in the frontend. |
| Playwright browsers | Chromium and Firefox for `@playwright/test` 1.63.0 | Browser tests. Install them with `pnpm exec playwright install --with-deps chromium firefox` in `web/`. The Chinese interface also needs a CJK font installed for screenshots to show text. |
| mdBook | 0.5 | Building the design book. |

The Linux desktop application has its own workspace in `desktop/` and also needs pkg-config, GTK 3, WebKitGTK 4.1, libsoup 3, librsvg, libayatana-appindicator and GStreamer with the base, good, bad and libav plugins, with their development headers. The NixOS VM checks need Nix and KVM and cannot run without them.

## Build

Keep the host's Cargo build-directory settings. Do not set `CARGO_TARGET_DIR` or add `target-dir` or `build-dir` to the repository's `.cargo/config.toml`.

`cargo build --locked -p neo-server` builds a server that serves only the API, which is enough for backend work with the Vite development server in front of it. The production server embeds the built frontend:

- With Nix, `nix build .#neo-server` produces `result/bin/kikoeru-neo` with the frontend embedded and the reduced ffmpeg supplied. `nix build .#neo-server-static` produces the fully static `x86_64` binary that releases publish.
- With Docker, `docker build -t kikoeru-neo .` builds the release image.

### Build the full server without Nix

The frontend build embeds two subtitle fallback fonts and their licenses. In the Dev Container and the Nix shell, `KIKOERU_FALLBACK_FONTS_DIR` already points at the built fonts, so the build is:

```sh
(cd web && pnpm install --frozen-lockfile && pnpm build)
mkdir -p web/build/licenses
cp "$KIKOERU_FALLBACK_FONTS_DIR/LICENSE-latin.txt" web/build/licenses/fallback-font-latin.txt
cp "$KIKOERU_FALLBACK_FONTS_DIR/LICENSE-cjk.txt" web/build/licenses/fallback-font-cjk.txt
cargo build --locked --release -p neo-server --features embed-web
```

With tools you installed yourself, build the fonts first. On Debian 13, install the distribution packages, then Rust through [rustup](https://rustup.rs), Node.js 22 from [nodejs.org](https://nodejs.org) (Debian 13 ships Node.js 20) and pnpm with `npm install -g pnpm@12.9.0`:

```sh
sudo apt install build-essential mold git curl python3 python3-fonttools \
  python3-brotli fonts-liberation fonts-noto-cjk ffmpeg
```

Then, from the repository root:

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

Other distributions install the same fonts under other paths; `fc-list | grep -E 'LiberationSans-Regular|NotoSansCJK-Regular'` shows where. `pnpm build` also generates the API types, which needs cargo. The result is `target/release/kikoeru-neo`, a single file with the frontend inside. It links against your system's libc, and it runs ffmpeg and ffprobe from `PATH` like the released binary.

## Run the backend and frontend

In your development environment, create a separate development data directory and configuration. Use synthetic media, never an existing installation's database or credentials:

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

In the Dev Container, write `listen = "0.0.0.0:8888"` instead. Account creation prompts for a password of at least 8 characters. On later runs, reuse the directory and configuration and run only `serve`. Stop the server with Ctrl-C.

In a second terminal in the same environment (another container terminal, or `nix develop` again at the repository root), start the frontend:

```sh
cd web
pnpm install --frozen-lockfile
pnpm gen:types
pnpm dev
```

Add `--host 0.0.0.0` to `pnpm dev` in the Dev Container. Open the URL Vite prints, normally `http://127.0.0.1:5173`. Vite forwards `/api` to `http://127.0.0.1:8888`. Sign in with the development account. To work on UI states without a backend, stop Vite and run:

```sh
pnpm dev:mock
```

Mock mode uses the JSON samples in `contract/`. It does not validate backend integration and must not be used for a production build. The interface is currently in Simplified Chinese.

## Run checks

Use focused checks while you work, then run the full set once before you open a pull request. The focused checks below run in any of the three environments. The full set needs Nix, which the Dev Container also contains. From the repository root:

```sh
nix flake check
nix build .#book --no-link
```

The `docker` and `nixos` checks start NixOS virtual machines and need `/dev/kvm`, which a container usually does not have. Without Nix, run the focused checks and let CI run the full set on your pull request. `nix flake check` builds the checks defined for the current system. The following checks are defined only on `x86_64-linux`: `rust-lint`, `rust-test`, `rust-test-pg`, `e2e`, `e2e-mock`, `windows`, `desktop`, `desktop-windows`, `docker` and `nixos`. The `neo-typegen` package's test phase also runs only on x86_64. A successful ARM64 check does not imply equivalent test coverage. GitHub CI runs the x86_64 check set.

`rust-test` runs the root Rust workspace with the embedded frontend. The desktop shell is a separate workspace. `rust-test-pg` runs selected database, scheduler and scanner targets, not every workspace test against PostgreSQL. Windows cross-checks prove compilation, not runtime behavior. Documentation-only changes are skipped by CI's path filters, so build the book locally.

Focused checks for the backend:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked -p neo-core --test contract
cargo test --locked -p neo-server --test api_task_admin
cargo test --locked -p neo-scanner
scripts/sqlx-prepare.sh --check
```

Replace the crate or test target with the one affected by your change. For the frontend, run from `web/`:

```sh
pnpm check
pnpm test
pnpm build
pnpm e2e:mock
```

Browser tests against the production server need a server with the embedded frontend. Build it with `nix build .#neo-server` or as described in [Build the full server without Nix](#build-the-full-server-without-nix), then run this in `web/` with the matching path:

```sh
KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e
KIKOERU_NEO_BIN=../target/release/kikoeru-neo pnpm e2e
```

The Nix shell and the Dev Container supply the Playwright browsers. Focused Cargo and pnpm checks do not replace the final flake checks. The [testing chapter](book/src/testing.md) is in Chinese and describes the coverage boundaries and hardware validation that automation cannot replace.

## Keep the code and specification consistent

The [design book](book/src/SUMMARY.md) is the implementation specification and is maintained in Chinese only. Read its introduction and the chapter for the area you change. The roadmap lists unfinished work by area, and the design-decisions page records established behavior and open questions. Update the relevant chapter in the same change when behavior or a design decision changes.

- The binding API contract is [the contract chapter](book/src/contract.md) (Chinese) plus the JSON samples in `contract/`. For a wire-format change, first make a separate commit touching only the chapter and samples, titled `docs: update API contract for ...`. Follow it with backend DTO and contract-test changes, then frontend changes. These commits may share one pull request.
- Rust DTOs in `neo-core::dto` generate TypeScript through ts-rs and `neo-typegen`. Run `pnpm gen:types` in `web/` to regenerate them. Do not hand-write API response types or commit generated files from `web/src/lib/api/types/`.
- Shared business SQL lives in `crates/neo-db/queries/` and is compiled for SQLite and PostgreSQL. Migrations, database copying, backend-specific statements and tests are exceptions, as defined in the [data-model chapter](book/src/data-model.md) (Chinese). Handlers, scans and imports call `neo_db` functions rather than adding SQL.
- After changing queries or migrations, run `scripts/sqlx-prepare.sh` in the development shell and include both backend crates' `.sqlx/` directories. The script uses temporary databases and removes them when finished.
- Add tests at the boundary whose behavior changes. Tests and screenshots use synthetic works, media and credentials. Never commit real library titles, paths, covers, database contents or configuration.

## Commits and pull requests

Keep each commit focused on one logical change. Use a subject of at most 72 characters in the form `<prefix>: <imperative description>`, with `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf` or `ci`. Add body bullets only when needed to explain the reason or a non-obvious constraint. End with your own name and email, not the maintainer's identity:

```text
fix: preserve progress when an older report arrives

- Ignore stale reports so concurrent devices cannot overwrite newer progress.

Signed-off-by: Your Name <you@example.com>
```

Open a pull request against `main` with what changed, why, and how you verified it. State skipped checks and known gaps. User-facing documentation is maintained in parallel under `docs/en/` and `docs/zh-CN/`, with same-language navigation. Keep the two versions consistent. Write book pages in Simplified Chinese and code comments in English.

CI must pass before merge whenever it runs. Fix failures rather than bypassing them. The maintainer may also run an AI-assisted correctness review. Verify each finding against a concrete trigger, fix confirmed bugs with regression coverage, and explain rejected findings. Contributors do not need an AI subscription or the maintainer's review setup. Merging and release publication remain maintainer actions.
