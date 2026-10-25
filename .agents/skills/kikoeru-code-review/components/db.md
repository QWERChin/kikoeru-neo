Component: database layer and migrations

Lenses: correctness, data

Files in scope:
- crates/neo-core/src/db/ (storage-layer types and traits)
- crates/neo-core/migrations/sqlite/ and crates/neo-core/migrations/postgres/
- crates/neo-db/ (queries/, src/, tests/)
- crates/neo-db-sqlite/src, crates/neo-db-postgres/src

Book chapters: data-model.md (query layout and database copying),
architecture.md and config.md (database selection). Operator steps for
`db copy` are in `docs/en/configuration.md`.

Look especially for: queries that return wrong rows (wrong JOIN, missing
WHERE, wrong GROUP BY, NULL handling, collation and case, LIKE escaping,
pagination with unstable ORDER BY, off-by-one offsets); SQLite and Postgres
versions of the same query that behave differently; transactions that do not
cover a read-modify-write, so concurrent requests lose updates; migrations
that lose data, cannot run on an existing production database, or leave the
two backends with different schemas; the SQLite-to-Postgres copy dropping or
mangling rows; timestamp and timezone handling; constraint violations a
normal flow can hit.
