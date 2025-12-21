//! Database Module
//! SQLite を使用した vendors/listings/receipts の管理

use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use anyhow::Result;
use tracing::info;

/// データベース接続プール
pub type DbPool = Pool<Sqlite>;

/// データベースを初期化
pub async fn init_db(db_path: &str) -> Result<DbPool> {
    // SQLite接続文字列
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    info!("Initializing database: {}", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // スキーマ作成
    create_schema(&pool).await?;

    info!("Database initialized successfully");
    Ok(pool)
}

/// スキーマ作成
async fn create_schema(pool: &DbPool) -> Result<()> {
    // runs テーブル（世代管理）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS runs (
            run_id TEXT PRIMARY KEY,
            env TEXT NOT NULL DEFAULT 'devnet',
            created_at_ms INTEGER NOT NULL
        )
    "#)
    .execute(pool)
    .await?;

    // vendors テーブル
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS vendors (
            stable_id TEXT PRIMARY KEY,
            latest_object_id TEXT,
            owner TEXT,
            mode INTEGER NOT NULL DEFAULT 0,
            manifest_url TEXT,
            manifest_sha256 TEXT,
            profile_seq INTEGER NOT NULL DEFAULT 0,
            status INTEGER NOT NULL DEFAULT 0,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            is_alive INTEGER NOT NULL DEFAULT 1
        )
    "#)
    .execute(pool)
    .await?;

    // artists テーブル（vendors と同型）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS artists (
            stable_id TEXT PRIMARY KEY,
            latest_object_id TEXT,
            owner TEXT,
            manifest_url TEXT,
            manifest_sha256 TEXT,
            profile_seq INTEGER NOT NULL DEFAULT 0,
            status INTEGER NOT NULL DEFAULT 0,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            is_alive INTEGER NOT NULL DEFAULT 1
        )
    "#)
    .execute(pool)
    .await?;

    // listings テーブル
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS listings (
            listing_id TEXT PRIMARY KEY,
            vendor_stable_id TEXT NOT NULL,
            vendor_object_id TEXT,
            seller TEXT,
            item_type INTEGER NOT NULL DEFAULT 0,
            item_id TEXT,
            price INTEGER NOT NULL,
            currency TEXT NOT NULL DEFAULT 'SUI',
            supply_total INTEGER NOT NULL DEFAULT 1,
            supply_remaining INTEGER NOT NULL DEFAULT 1,
            status INTEGER NOT NULL DEFAULT 0,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            is_alive INTEGER NOT NULL DEFAULT 1,
            FOREIGN KEY (vendor_stable_id) REFERENCES vendors(stable_id)
        )
    "#)
    .execute(pool)
    .await?;

    // receipts テーブル
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS receipts (
            receipt_id TEXT PRIMARY KEY,
            vendor_stable_id TEXT NOT NULL,
            listing_id TEXT NOT NULL,
            buyer TEXT NOT NULL,
            qty INTEGER NOT NULL DEFAULT 1,
            price INTEGER NOT NULL,
            currency TEXT NOT NULL DEFAULT 'SUI',
            timestamp_ms INTEGER NOT NULL,
            tx_digest TEXT,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT
        )
    "#)
    .execute(pool)
    .await?;

    // tombstones テーブル（死亡オブジェクト管理）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS tombstones (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            stable_id TEXT,
            object_id TEXT NOT NULL,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT,
            observed_dead_at_ms INTEGER NOT NULL,
            note TEXT
        )
    "#)
    .execute(pool)
    .await?;

    // インデックス作成
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_vendors_is_alive ON vendors(is_alive)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_listings_vendor ON listings(vendor_stable_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_listings_is_alive ON listings(is_alive)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_receipts_buyer ON receipts(buyer)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_receipts_listing ON receipts(listing_id)")
        .execute(pool).await?;

    Ok(())
}
