use sqlx::any::AnyPoolOptions;

/// Postgres 测试需显式提供 TEST_DATABASE_URL（postgres://...），否则跳过。
fn pg_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|u| u.starts_with("postgres"))
}

/// 证明 Any 驱动会把 `?` 占位符改写为 `$1`，并能正确解码 i64 / String / Option<String>。
#[tokio::test]
async fn spike_any_postgres_placeholders_and_types() {
    let Some(url) = pg_url() else {
        eprintln!("skipping spike_any_postgres_placeholders_and_types: TEST_DATABASE_URL not set");
        return;
    };
    share_secret::db::install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect postgres");

    sqlx::query("DROP TABLE IF EXISTS spike_t")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE spike_t (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL, note TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // INSERT with `?` placeholders — Any must rewrite to $1, $2 for Postgres.
    sqlx::query("INSERT INTO spike_t (name, note) VALUES (?, ?)")
        .bind("alice")
        .bind(Option::<String>::None)
        .execute(&pool)
        .await
        .expect("insert with ? placeholders");

    // SELECT back with a `?` placeholder and decode i64 + String + Option<String>.
    let row: (i64, String, Option<String>) =
        sqlx::query_as("SELECT id, name, note FROM spike_t WHERE name = ?")
            .bind("alice")
            .fetch_one(&pool)
            .await
            .expect("select and decode");

    assert!(row.0 >= 1);
    assert_eq!(row.1, "alice");
    assert_eq!(row.2, None);

    sqlx::query("DROP TABLE spike_t").execute(&pool).await.unwrap();
}
