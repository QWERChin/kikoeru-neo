Component: server startup, configuration, and backup

Lenses: correctness, data

Files in scope (crates/neo-server/src/): main.rs, cli.rs, serve.rs,
serve/killable.rs, config.rs, datadir.rs, logging.rs, backup.rs, and their
tests in crates/neo-server/tests/ (cli.rs, migrate.rs, process/,
common/).

Book chapters: config.md, deployment.md, architecture.md.

Look especially for: config keys whose default, parsing, or precedence (file,
environment, CLI flags) differs from config.md; settings that are read once
at startup but the spec says can change at runtime, or the reverse; startup
that runs migrations or touches data before checks the spec requires (for
example refusing to start on a database with unknown migrations); graceful
shutdown that drops in-flight writes, leaves tasks in running, or leaves
child processes alive; backups that copy a SQLite database in an
inconsistent state, overwrite the previous good backup before the new one is
complete, or rotate the wrong files; data directory permission and layout
handling that breaks on an upgrade from an older layout; log rotation or
filtering that loses logs.
