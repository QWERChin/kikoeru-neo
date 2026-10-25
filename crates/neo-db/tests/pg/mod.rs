//! A throwaway PostgreSQL database per test. `KIKOERU_TEST_PG` names the
//! server, as a URL whose user may create databases. Tests that use it are
//! ignored by default and run with `cargo test -- --ignored`.

use std::{
    str::FromStr,
    sync::atomic::{AtomicU32, Ordering},
};

use sqlx::{
    ConnectOptions, Connection, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

/// A new, empty database and its URL. Each test gets its own, so tests
/// running at the same time never see each other's rows.
pub async fn fresh() -> (PgPool, String) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let url = std::env::var("KIKOERU_TEST_PG")
        .expect("set KIKOERU_TEST_PG to a throwaway PostgreSQL server's URL");
    let admin = PgConnectOptions::from_str(&url).expect("KIKOERU_TEST_PG is not a valid URL");
    let name = format!(
        "kneo_test_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut conn = PgConnection::connect_with(&admin).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    let options = admin.database(&name);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    (pool, options.to_url_lossy().to_string())
}
