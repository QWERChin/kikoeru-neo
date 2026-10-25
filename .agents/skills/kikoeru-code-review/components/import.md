Component: legacy import and database migration commands

Lenses: correctness, data

Files in scope (crates/neo-server/src/): import/ (all files), migrate_cmd.rs,
and their tests in crates/neo-server/tests/ (import_legacy.rs,
import_legacy_versions.rs, import_merge.rs, contract_import.rs, migrate.rs,
fixtures/).

Book chapters: migration.md (legacy format and configuration mapping),
data-model.md, testing.md (import coverage).

The old kikoeru-express format is documented in `book/src/migration.md`.
Use that chapter and the synthetic fixtures to check legacy semantics.

Look especially for: mapping rules that drop or mis-map fields (users,
favorites, progress, reviews, playlists, tags, circles, vas, history),
especially across old versions; merges of reviews and history that lose the
newer value; bad JSON or out-of-range indexes in old data that abort the
whole import or silently corrupt a row; non-empty targets that are not
rejected; partial failure that leaves a half-imported database marked as
complete; re-matching (unique, ambiguous, superseded) that assigns old data
to the wrong work, including works with the same number under different
prefixes; the interactive migrate command choosing different mappings or
import modes from import-legacy. SQLite/PostgreSQL copying belongs to
`db copy` and the database component review.
