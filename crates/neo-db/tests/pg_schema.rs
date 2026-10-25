#![cfg(feature = "postgres")]
//! The PostgreSQL migrations must build the same schema as the SQLite ones
//! (book/src/data-model.md, "PostgreSQL"). These tests need a throwaway
//! server, see `pg/mod.rs`.

mod pg;

use std::collections::{BTreeMap, BTreeSet};

use neo_db::{Db, PG_MIGRATOR};
use sqlx::{PgPool, Row};

/// SQLite INTEGER columns that hold a flag. PostgreSQL stores them as
/// BOOLEAN, so that the same `AS "x: bool"` reads them on both.
const FLAGS: [(&str, &str); 5] = [
    ("works", "nsfw"),
    ("users", "is_admin"),
    ("play_progress", "replaying"),
    ("legacy_user_state", "has_review"),
    ("user_settings", "normalize_loudness"),
];

#[derive(Debug, PartialEq, Eq)]
struct Column {
    name: String,
    /// `integer`, `boolean`, `text`, `real` or `blob`.
    class: &'static str,
    not_null: bool,
    default: Option<String>,
}

/// Everything about one table that a query or a copy between the two
/// backends can tell apart.
#[derive(Debug, Default, PartialEq, Eq)]
struct Table {
    columns: Vec<Column>,
    primary_key: Vec<String>,
    /// Column lists of UNIQUE constraints and unique indexes, without the
    /// primary key, and whether the index is partial. The predicates are
    /// written differently on each side, so
    /// `active_task_uniqueness_is_the_same_on_both` checks what they do.
    unique: BTreeSet<(Vec<String>, bool)>,
    /// Named, non-unique indexes, their columns and whether they are partial.
    indexes: BTreeMap<String, (Vec<String>, bool)>,
    /// (columns, referenced table, referenced columns, ON DELETE action).
    foreign_keys: BTreeSet<(Vec<String>, String, Vec<String>, String)>,
}

/// Strips quotes, casts and the case of TRUE and FALSE, so that SQLite's
/// `0` and `'none'` compare equal to PostgreSQL's `false` and
/// `'none'::text`.
fn normalize_default(raw: &str, class: &str) -> String {
    let mut value = raw.trim();
    if let Some(cast) = value.find("::") {
        value = &value[..cast];
    }
    let value = value.trim_matches('\'').to_ascii_lowercase();
    match (class, value.as_str()) {
        ("boolean", "0" | "false") => "false".to_owned(),
        ("boolean", "1" | "true") => "true".to_owned(),
        _ => value,
    }
}

async fn sqlite_schema(db: &Db) -> BTreeMap<String, Table> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table'
           AND name NOT LIKE 'sqlite%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&db.sqlite().unwrap().read)
    .await
    .unwrap();
    let mut tables = BTreeMap::new();
    for name in names {
        let mut table = Table::default();
        let mut key = Vec::new();
        for row in sqlx::query("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info($1) ORDER BY cid")
            .bind(&name)
            .fetch_all(&db.sqlite().unwrap().read)
            .await
            .unwrap()
        {
            let column: String = row.get("name");
            let declared: String = row.get("type");
            let pk: i64 = row.get("pk");
            let class = match declared.as_str() {
                "INTEGER" if FLAGS.contains(&(name.as_str(), column.as_str())) => "boolean",
                "INTEGER" => "integer",
                "TEXT" => "text",
                "REAL" => "real",
                "BLOB" => "blob",
                other => panic!("{name}.{column} has an unexpected type {other}"),
            };
            if pk > 0 {
                key.push((pk, column.clone()));
            }
            table.columns.push(Column {
                // SQLite lets a primary key column hold NULL unless it says
                // NOT NULL. PostgreSQL never does, and no row has one.
                not_null: row.get::<i64, _>("notnull") == 1 || pk > 0,
                default: row
                    .get::<Option<String>, _>("dflt_value")
                    .map(|d| normalize_default(&d, class)),
                name: column,
                class,
            });
        }
        key.sort();
        table.primary_key = key.into_iter().map(|(_, c)| c).collect();
        let indexes = sqlx::query(
            "SELECT name, \"unique\", partial FROM pragma_index_list($1) WHERE origin <> 'pk'",
        )
        .bind(&name)
        .fetch_all(&db.sqlite().unwrap().read)
        .await
        .unwrap();
        for index in indexes {
            let index_name: String = index.get("name");
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_index_info($1) ORDER BY seqno")
                    .bind(&index_name)
                    .fetch_all(&db.sqlite().unwrap().read)
                    .await
                    .unwrap();
            let partial = index.get::<i64, _>("partial") == 1;
            if index.get::<i64, _>("unique") == 1 {
                table.unique.insert((columns, partial));
            } else {
                table.indexes.insert(index_name, (columns, partial));
            }
        }
        let mut keys: BTreeMap<i64, (Vec<String>, String, Vec<String>, String)> = BTreeMap::new();
        for fk in sqlx::query(
            "SELECT id, \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list($1)
             ORDER BY id, seq",
        )
        .bind(&name)
        .fetch_all(&db.sqlite().unwrap().read)
        .await
        .unwrap()
        {
            let entry = keys.entry(fk.get("id")).or_insert_with(|| {
                (
                    Vec::new(),
                    fk.get("table"),
                    Vec::new(),
                    fk.get::<String, _>("on_delete").to_ascii_uppercase(),
                )
            });
            entry.0.push(fk.get("from"));
            entry.2.push(fk.get("to"));
        }
        table.foreign_keys = keys.into_values().collect();
        tables.insert(name, table);
    }
    tables
}

async fn pg_schema(pool: &PgPool) -> BTreeMap<String, Table> {
    let mut tables: BTreeMap<String, Table> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT table_name::text AS t, column_name::text AS c, data_type::text AS ty,
                is_nullable = 'NO' AS not_null, column_default::text AS d
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations'
         ORDER BY table_name, ordinal_position",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    {
        let table: String = row.get("t");
        let column: String = row.get("c");
        let ty: String = row.get("ty");
        let class = match ty.as_str() {
            "bigint" => "integer",
            "boolean" => "boolean",
            "text" => "text",
            "double precision" => "real",
            "bytea" => "blob",
            other => panic!("{table}.{column} has an unexpected type {other}"),
        };
        tables.entry(table).or_default().columns.push(Column {
            default: row
                .get::<Option<String>, _>("d")
                .map(|d| normalize_default(&d, class)),
            not_null: row.get("not_null"),
            name: column,
            class,
        });
    }
    // Every index with its columns in key order. The primary key is the
    // one backing the table's PRIMARY KEY constraint.
    for row in sqlx::query(
        "SELECT t.relname::text AS t, i.relname::text AS i, x.indisunique AS u,
                x.indisprimary AS p, x.indpred IS NOT NULL AS partial, c.contype::text AS ct,
                array_agg(a.attname::text ORDER BY k.n) AS cols
         FROM pg_index x
         JOIN pg_class t ON t.oid = x.indrelid
         JOIN pg_class i ON i.oid = x.indexrelid
         JOIN pg_namespace s ON s.oid = t.relnamespace
         LEFT JOIN pg_constraint c ON c.conindid = x.indexrelid
         CROSS JOIN LATERAL unnest(x.indkey) WITH ORDINALITY AS k(attnum, n)
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
         WHERE s.nspname = 'public' AND t.relname <> '_sqlx_migrations'
         GROUP BY t.relname, i.relname, x.indisunique, x.indisprimary, x.indpred, c.contype",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    {
        let table = tables.get_mut(&row.get::<String, _>("t")).unwrap();
        let columns: Vec<String> = row.get("cols");
        let partial: bool = row.get("partial");
        if row.get("p") {
            table.primary_key = columns;
        } else if row.get("u") {
            table.unique.insert((columns, partial));
        } else {
            table.indexes.insert(row.get("i"), (columns, partial));
        }
    }
    for row in sqlx::query(
        "SELECT t.relname::text AS t, r.relname::text AS r,
                ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY AS k(n, o)
                      JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n
                      ORDER BY k.o) AS cols,
                ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY AS k(n, o)
                      JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.n
                      ORDER BY k.o) AS refs,
                CASE c.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL'
                     WHEN 'r' THEN 'RESTRICT' WHEN 'd' THEN 'SET DEFAULT'
                     ELSE 'NO ACTION' END AS on_delete
         FROM pg_constraint c
         JOIN pg_class t ON t.oid = c.conrelid
         JOIN pg_class r ON r.oid = c.confrelid
         JOIN pg_namespace s ON s.oid = t.relnamespace
         WHERE c.contype = 'f' AND s.nspname = 'public'",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    {
        let table = tables.get_mut(&row.get::<String, _>("t")).unwrap();
        table.foreign_keys.insert((
            row.get("cols"),
            row.get("r"),
            row.get("refs"),
            row.get("on_delete"),
        ));
    }
    tables
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn postgres_migrations_build_the_sqlite_schema() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let (pg, _) = pg::fresh().await;
    PG_MIGRATOR.run(&pg).await.unwrap();

    let want = sqlite_schema(&sqlite).await;
    let got = pg_schema(&pg).await;
    assert_eq!(
        want.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>(),
        "the two backends have different tables"
    );
    for (name, table) in &want {
        assert_eq!(&got[name], table, "table {name} differs");
    }
    sqlite.close().await;
    pg.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn both_backends_end_at_the_same_migration() {
    let last = |m: &sqlx::migrate::Migrator| m.iter().map(|m| m.version).max();
    assert_eq!(last(&PG_MIGRATOR), last(&neo_db::MIGRATOR));
    let (pg, _) = pg::fresh().await;
    PG_MIGRATOR.run(&pg).await.unwrap();
    // Running them again changes nothing, like a restart of the server.
    PG_MIGRATOR.run(&pg).await.unwrap();
    pg.close().await;
}

/// `tasks_active_dedupe` lets a key repeat among finished tasks but not
/// among queued and running ones. Run the same statements on both.
#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn active_task_uniqueness_is_the_same_on_both() {
    const INSERT: &str =
        "INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
                          VALUES ('scan', 'k', $1, '{}', 0, 0)";
    let dir = tempfile::tempdir().unwrap();
    let sqlite = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let (pg, _) = pg::fresh().await;
    PG_MIGRATOR.run(&pg).await.unwrap();
    for (state, allowed) in [
        ("done", true),
        ("failed", true),
        ("cancelled", true),
        ("queued", true),
        ("running", false),
        ("queued", false),
        ("done", true),
    ] {
        let on_sqlite = sqlx::query(INSERT)
            .bind(state)
            .execute(&sqlite.sqlite().unwrap().write)
            .await;
        let on_pg = sqlx::query(INSERT).bind(state).execute(&pg).await;
        assert_eq!(on_sqlite.is_ok(), allowed, "SQLite, {state}");
        assert_eq!(on_pg.is_ok(), allowed, "PostgreSQL, {state}");
    }
    sqlite.close().await;
    pg.close().await;
}
