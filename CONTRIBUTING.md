# Contributing to kikoeru-neo

Use the Nix development environment to build the Rust backend and Svelte frontend and test changes with synthetic data. You need Git and Nix with flakes enabled on `x86_64-linux` or `aarch64-linux`. For installation rather than development, start with the [user guide](docs/en/index.md).

## Set up and build

From your checkout:

```sh
nix develop
cargo build --locked -p neo-server
nix build .#neo-server
```

The shell supplies Rust, mold, Node, pnpm, ffmpeg, ffprobe, mdBook, SQLx CLI and PostgreSQL. The Cargo build serves the API without a frontend. The Nix package at `result/bin/kikoeru-neo` embeds the production frontend and supplies ffmpeg and ffprobe.

Keep the host's Cargo build-directory settings. Do not set `CARGO_TARGET_DIR` or add `target-dir` or `build-dir` to the repository's `.cargo/config.toml`.

## Run the backend and frontend

In the development shell, create a separate development data directory and configuration. Use synthetic media, never an existing installation's database or credentials:

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

Account creation prompts for a password of at least 8 characters. On later runs, reuse the directory and configuration and run only `serve`. Stop the server with Ctrl-C.

In a second terminal, enter `nix develop` at the repository root, then start the frontend:

```sh
cd web
pnpm install --frozen-lockfile
pnpm gen:types
pnpm dev
```

Open the URL Vite prints, normally `http://127.0.0.1:5173`. Vite forwards `/api` to `http://127.0.0.1:8888`. Sign in with the development account. To work on UI states without a backend, stop Vite and run:

```sh
pnpm dev:mock
```

Mock mode uses the JSON samples in `contract/`. It does not validate backend integration and must not be used for a production build. The interface is currently in Simplified Chinese.

## Run checks

From the repository root:

```sh
nix flake check
nix build .#book --no-link
```

`nix flake check` builds the checks defined for the current system. The following checks are defined only on `x86_64-linux`: `rust-lint`, `rust-test`, `rust-test-pg`, `e2e`, `e2e-mock`, `windows`, `desktop`, `desktop-windows`, `docker` and `nixos`. The `neo-typegen` package's test phase also runs only on x86_64. A successful ARM64 check does not imply equivalent test coverage. GitHub CI runs the x86_64 check set.

`rust-test` runs the root Rust workspace with the embedded frontend. The desktop shell is a separate workspace. `rust-test-pg` runs selected database and scanner targets, not every workspace test against PostgreSQL. Windows cross-checks prove compilation, not runtime behavior. Documentation-only changes are skipped by CI's path filters, so build the book locally.

During development, use focused checks inside `nix develop`:

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

For production-package browser tests, first build `nix build .#neo-server` at the root, then run this in `web/` inside the development shell:

```sh
KIKOERU_NEO_BIN=../result/bin/kikoeru-neo pnpm e2e
```

The development shell supplies the Playwright browsers. Focused Cargo and pnpm checks do not replace the final flake checks. The [testing chapter](book/src/testing.md) is in Chinese and describes the coverage boundaries and hardware validation that automation cannot replace.

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
