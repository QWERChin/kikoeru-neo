Component: desktop app

Lenses: correctness, auth, data

Files in scope: desktop/src-tauri/ (src, Cargo.toml, tauri.conf.json,
tauri.bundle.conf.json, capabilities/, build.rs, Info.plist),
desktop/e2e/run.mjs, web/src/lib/desktop/, web/src/routes/desktop/, and the
server side of the shell in crates/neo-server/src/embed.rs with its test
crates/neo-server/tests/embed.rs.

Book chapters: desktop.md (architecture and packaging), config.md and
storage.md (local roots). Installation steps are in docs/en/desktop.md.

Look especially for: local mode in embed.rs (creating the local admin,
issuing a session token without a password, merging legacy data) doing
something the book does not intend; the embedded server's lifecycle (port selection races,
a second instance starting a second server on the same data directory,
server left running after the window or tray quits, startup failure that
leaves the window blank); IPC commands reachable from the web view that
should not be (capabilities broader than needed, remote origins allowed),
which matters because the web view renders library metadata; local root
picking and path handling on Windows and macOS (UNC paths, drive letters,
Unicode normalization, security-scoped bookmarks); config files written
non-atomically; the import flow from an existing server; ffmpeg discovery
and fallback; log file handling.
