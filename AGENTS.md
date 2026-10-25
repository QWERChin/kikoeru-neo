# kikoeru-neo: agent rules

kikoeru-neo is a self-hosted player for audio dramas and audiobooks with
DLsite metadata. It is an independent rewrite that reads the old
kikoeru-express data format. The backend uses Rust, axum, tokio and SQLx
with SQLite and PostgreSQL. The static Svelte 5 frontend is embedded in the
server and shared with the Tauri desktop app. Nix builds the project.

Read these rules, then `book/src/SUMMARY.md` and the chapters relevant to
the task. Human contributors start with `CONTRIBUTING.md` or
`CONTRIBUTING.zh-CN.md`.

## Find the specification and code

- `book/src/` is the implementation specification, in Simplified Chinese.
  Start with `introduction.md`. `roadmap.md` lists unfinished work by
  area, not milestones or agent assignments. `open-questions.md` records
  design decisions and open product questions. Change established behavior
  only within the task's scope, with matching specification and tests.
- `docs/en/` and `docs/zh-CN/` are parallel user guides. Keep navigation in
  the page's language. Label references to the Chinese-only design book
  from English contributor or tooling documentation.
- `book/src/contract.md` and `contract/` define the binding API contract.
  `book/src/api.md` explains API design without redefining fields.
- `crates/neo-core/src/dto/` defines API types. `neo-typegen` exports them
  through ts-rs into `web/src/lib/api/types/`. Generate these files rather
  than editing or committing them. Never hand-write API response types.
- Shared business SQL lives in `crates/neo-db/queries/`, compiled by
  `neo-db-sqlite` and `neo-db-postgres`. Migrations, database copying,
  backend-specific statements and tests are explicit exceptions. Read
  `book/src/data-model.md` before changing these boundaries. Handlers,
  scans and imports call `neo_db` functions instead of writing SQL.
- `web/` owns the browser frontend and shared desktop UI.
  `desktop/src-tauri/` is a separate Rust workspace for the desktop shell.
- `flake.nix`, `nix/` and `.github/workflows/` define packaging and checks.
  `scripts/` holds their supporting tools.
- `book/src/migration.md` documents the legacy data format and import
  rules. No separate source checkout is required to contribute.
- `openlist-client` is pinned by revision in `Cargo.toml` from
  `https://github.com/chinrw/openlist-client`. Propose dependency changes
  in that project before updating the pin here.

## Keep changes coherent

The book comes first. When an implementation needs a new decision or must
differ from the spec, update the relevant chapter in the same change and
explain the reason. Keep related chapters and user guides consistent.
Update a roadmap item when its acceptance conditions are actually met.

For API changes, make a separate commit touching only
`book/src/contract.md` and `contract/`, titled
`docs: update API contract for ...`. Follow it with the backend DTO and
contract tests, then the frontend. These commits may share a pull request.
Contract tests compare complete JSON values, including nulls and sample
values. Do not agree on fields outside the contract.

After changing a query macro or migration, run `scripts/sqlx-prepare.sh`
in `nix develop` and include both backend crates' `.sqlx/` directories.
The data-model chapter describes portable SQL and backend exceptions.

Preserve host-level Cargo build-directory settings. Do not set
`CARGO_TARGET_DIR` or add `target-dir` or `build-dir` to the repository's
`.cargo/config.toml`. Before removing a merged task worktree, run
`cargo clean` there so its build output is removed too.

## Coordinate parallel work

Use one task branch per worktree, based on current `origin/main`, when the
task authorizes branch creation. Check existing worktrees and branches
before starting, and preserve another contributor's work. Do not rebase
or edit someone else's branch without coordination.

When splitting a task between agents, agree on file ownership first:

- The backend owner handles `crates/`, SQLx metadata, `desktop/src-tauri/`,
  `flake.nix` and `nix/`.
- The frontend owner handles `web/` and uses contract samples as mock data
  until the backend is available.
- Assign scripts to the owner of the workflow they support. Assign shared
  book and contract edits explicitly so agents do not edit them together.
- Integration combines both sides and verifies the production package and
  end-to-end behavior. Keep cross-boundary edits scoped to integration.

## Verify and review

Run the checks described in `CONTRIBUTING.md` and `book/src/testing.md`.
Use focused checks during development, then `nix flake check` for the final
change. Report skipped checks and platform gaps. The full Rust, browser,
Windows cross-compilation and VM check set is defined only on x86_64 Linux.
Build changed book pages with `nix build .#book --no-link` and check links.
CI skips documentation-only paths, so those checks remain necessary locally.

CI sandboxes may have no `/usr/bin/env`, and `/homeless-shelter` is not a
writable home. Use `#!/bin/sh` for portable test fixtures. Checks that write
under the home directory must set `HOME` to their temporary directory.

Pull requests explain what changed, why and how it was verified. CI must
pass whenever it runs. The maintainer may request an AI-assisted review
using `.agents/skills/kikoeru-code-review/SKILL.md`. Review concrete
correctness defects, verify triggers, add regression tests for confirmed
bugs and explain rejected findings. A review is evidence, not permission
to push, merge, publish or deploy. Perform those actions only within the
requester's authorization and repository permissions. Do not bypass failing
checks or extend a review into unrequested changes.

## Protect data

Use synthetic media and disposable databases for development and tests.
Never commit real library data, titles, track names, covers, credentials or
installation configuration. Do not run tests against a real installation.
Access to a checkout does not authorize changing a running service or its
data. Read-only inspection of an installation, when authorized, must use a
consistent database snapshot rather than copying live database files.

## Write for the next contributor

- Write book pages in Simplified Chinese, in complete sentences, without
  an AI-assistant tone or emoji. Use at most one em dash per paragraph and
  no semicolons joining clauses.
- Write code comments in English. Explain decisions and constraints rather
  than restating the code, usually in one to three lines.
- Keep each commit focused. Subjects use `<prefix>: <imperative description>`
  and at most 72 characters. Prefixes are `feat`, `fix`, `refactor`, `docs`,
  `test`, `chore`, `perf` and `ci`. Use body bullets for reasons the subject
  cannot explain. The final line is `Signed-off-by: Name <email>` using the
  contributor's own identity. Do not add agent attribution or co-author trailers.
- The license is GPL-3.0-or-later. Keep `LICENSE` unchanged.
