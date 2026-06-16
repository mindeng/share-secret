use sqlx::any::AnyPoolOptions;
use sqlx::AnyPool;
use std::env;
use std::sync::Once;

static DRIVERS: Once = Once::new();

/// 进程内只安装一次 Any 驱动（重复安装会报错）。init_db/测试都通过它来安装。
pub fn install_drivers_once() {
    DRIVERS.call_once(|| {
        sqlx::any::install_default_drivers();
    });
}

fn is_postgres(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

pub async fn init_db() -> AnyPool {
    install_drivers_once();
    let mut database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:share_secret.db".to_string());

    let postgres = is_postgres(&database_url);

    // SQLite 默认不会自动建库文件；补上 mode=rwc 以保持原有 create_if_missing 行为。
    if !postgres && !database_url.contains(":memory:") && !database_url.contains("mode=") {
        let sep = if database_url.contains('?') { '&' } else { '?' };
        database_url = format!("{database_url}{sep}mode=rwc");
    }

    let pool = AnyPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to database");

    if postgres {
        init_postgres_schema(&pool).await;
    } else {
        init_sqlite_schema(&pool).await;
    }

    pool
}

pub async fn init_db_memory() -> AnyPool {
    install_drivers_once();
    // 必须用单连接：`sqlite::memory:` 每条连接都是独立的内存库，
    // 多连接会导致建表的连接和后续查询的连接不是同一个库（no such table）。
    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory sqlite");

    init_sqlite_schema(&pool).await;
    pool
}

/// 每条语句单独执行：Any 驱动不保证支持单次调用里的多语句。
pub async fn init_sqlite_schema(pool: &AnyPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create shares table");

    // 迁移旧库：若 kdf_salt 列不存在则补上（已存在会报错，忽略即可）。
    let _ = sqlx::query("ALTER TABLE shares ADD COLUMN kdf_salt TEXT")
        .execute(pool)
        .await;
}

pub async fn init_postgres_schema(pool: &AnyPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (now())::text
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shares (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id),
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at TEXT NOT NULL DEFAULT (now())::text
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create shares table");
}
