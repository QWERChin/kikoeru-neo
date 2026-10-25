# Lens: data

Active when the reviewed code touches migrations, SQL, the storage-layer
types, the database copy, legacy import, backups, or anything that deletes
or rewrites stored rows or files. Owns data safety on a real production
database: losing, corrupting, or silently rewriting what the user already
has.

The spec's rules to check against (`book/src/data-model.md`):

- Shared business SQL lives in `crates/neo-db/queries/` and compiles for
  SQLite and PostgreSQL. Migrations, database copying, backend-specific
  statements and tests are exceptions documented in `book/src/data-model.md`.
  The two schemas must stay equivalent; every
  migration after 9 exists in both `migrations/sqlite/` and
  `migrations/postgres/` with the same number.
- Only portable SQL: `$1` placeholders, `TRUE`/`FALSE`, `IS NOT DISTINCT
  FROM` for nullable equality, `ON CONFLICT ... DO UPDATE`, `RETURNING`,
  SQL `lower()` on the searched column with ASCII-lowercased input from
  Rust (SQLite `LIKE` is case-insensitive for ASCII, PostgreSQL's is not).
  PostgreSQL text uses
  `COLLATE "C"`. The same statement must read the same Rust types on both.
- SQLite: one write connection, short write transactions, no network or
  ffmpeg work inside one. `retry_busy` wraps only whole, side-effect-free
  transactions.
- A failed migration stops the server. The database copy writes everything
  in one transaction, checks row counts, and advances PostgreSQL sequences.

Hunt for:

1. A migration that cannot run on an existing production database (data
   that violates a new constraint, a rebuilt table that drops rows or
   foreign keys, a default that rewrites user data), or that leaves the two
   backends with different schemas or semantics.
2. A query whose result differs between SQLite and PostgreSQL (NULL
   ordering, collation, integer division, boolean reads, `LIKE` case,
   `GROUP BY` leniency in SQLite).
3. A delete or overwrite that a normal flow can trigger by mistake: a scan
   that marks or removes works after a partial failure, a metadata refresh
   that overwrites user edits, an import or copy that drops rows or
   reassigns legacy data to the wrong work.
4. A read-modify-write split across transactions, so concurrent writers
   lose an update.
5. A backup that can capture an inconsistent database or replace the last
   good backup before the new one is complete.
