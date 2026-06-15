use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::env;
use std::str::FromStr;

pub async fn init_db() -> SqlitePool {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:share_secret.db".to_string());
    let options = SqliteConnectOptions::from_str(&database_url)
        .expect("invalid DATABASE_URL")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .expect("failed to connect to sqlite");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        "#,
    )
    .execute(&pool)
    .await
    .expect("failed to create tables");

    // 迁移旧数据库：若 kdf_salt 列不存在则补上（已存在会报错，忽略即可）
    let _ = sqlx::query("ALTER TABLE shares ADD COLUMN kdf_salt TEXT")
        .execute(&pool)
        .await;

    pool
}

pub async fn init_db_memory() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        "#,
    )
    .execute(&pool)
    .await
    .expect("failed to create tables");

    pool
}
