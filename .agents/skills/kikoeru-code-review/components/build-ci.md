Component: Nix packaging, NixOS module, Docker images, CI, and end-to-end tests

Lenses: correctness

Files in scope: flake.nix, flake.lock (only for pinned inputs), nix/,
.github/workflows/, scripts/ (shell and Python scripts and their bats tests,
scripts/ios/), web/e2e/, web/e2e-mock/, web/playwright.config.ts,
web/playwright.mock.config.ts, web/vite.config.ts, web/package.json.

Book chapters: deployment.md, testing.md, stack.md.

Look especially for: the NixOS module producing a service that cannot start,
cannot write its data directory, or restarts during a scan; option defaults
and types that differ from docs/en/configuration.md; a package that embeds a stale or
missing frontend, or the wrong ffmpeg; `nix flake check` claiming to run a
check that is not actually wired in; Docker migration and GHCR release
scripts that copy data or publish the wrong image, or that the bats tests
would not catch; CI workflows that skip jobs on the branches that matter,
cache across incompatible inputs, or report success when a step failed
(pipes without pipefail, `|| true`); end-to-end tests that pass while the
feature is broken (assertions that cannot fail, waits that hide races,
mocks that diverge from the real API).
