#!/usr/bin/env bash
# Regenerates the query caches of both database backends
# (crates/neo-db-sqlite/.sqlx and crates/neo-db-postgres/.sqlx) against
# throwaway databases. Run it inside `nix develop` after changing a query
# or a migration. `--check` only reports whether the caches are current.
set -euo pipefail
cd "$(dirname "$0")/.."
check=()
if [[ "${1:-}" == "--check" ]]; then
  check=(--check)
fi

# A short directory: PostgreSQL's socket path must stay under 108 bytes.
work=$(mktemp -d /tmp/neo-sqlx.XXXXXX)
trap 'pg_ctl -D "$work/pg" -m fast stop >/dev/null 2>&1 || true; rm -rf "$work"' EXIT

export DATABASE_URL="sqlite:$work/neo.sqlite3"
sqlx database create
sqlx migrate run --source crates/neo-core/migrations/sqlite >/dev/null
(cd crates/neo-db-sqlite && SQLX_OFFLINE=false cargo sqlx prepare "${check[@]}" -- --locked)
unset DATABASE_URL

initdb -D "$work/pg" -U postgres --auth=trust --no-locale -E UTF8 >/dev/null
pg_ctl -D "$work/pg" -o "-k $work -c listen_addresses=" -l "$work/pg.log" -w start >/dev/null
export PG_DATABASE_URL="postgres:///neo?host=$work&user=postgres"
sqlx database create --database-url "$PG_DATABASE_URL"
sqlx migrate run --source crates/neo-core/migrations/postgres --database-url "$PG_DATABASE_URL" >/dev/null
(cd crates/neo-db-postgres && SQLX_OFFLINE=false cargo sqlx prepare "${check[@]}" -- --locked)
