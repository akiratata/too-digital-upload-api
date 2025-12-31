//! Database Module
//! SQLite を使用した vendors/listings/receipts/artists の管理

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

    // vendors テーブル（peer_id + shop_type 対応）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS vendors (
            stable_id TEXT PRIMARY KEY,
            peer_id TEXT,
            peer_id_sha256 TEXT,
            latest_object_id TEXT,
            owner TEXT,
            mode INTEGER NOT NULL DEFAULT 0,
            shop_type INTEGER NOT NULL DEFAULT 0,
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

    // vendors の peer_id インデックス
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_vendors_peer_id ON vendors(peer_id)")
        .execute(pool).await.ok();  // 既存テーブルでは失敗してもOK

    // artists テーブル（peer_id 対応）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS artists (
            stable_id TEXT PRIMARY KEY,
            peer_id TEXT NOT NULL,
            peer_id_sha256 TEXT,
            latest_object_id TEXT,
            owner TEXT,
            profile_url TEXT,
            profile_sha256 TEXT,
            discography_url TEXT,
            discography_sha256 TEXT,
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

    // artists の peer_id ユニーク インデックス
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_artists_peer_id ON artists(peer_id)")
        .execute(pool).await?;

    // discography テーブル（アーティスト ↔ アルバム紐付け）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS discography (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            artist_stable_id TEXT NOT NULL,
            album_id TEXT NOT NULL,
            edition_id TEXT,
            title TEXT,
            cover_thumb_url TEXT,
            track_count INTEGER NOT NULL DEFAULT 0,
            track_preview TEXT,
            role TEXT NOT NULL DEFAULT 'main',
            deployed_at_ms INTEGER,
            created_at_ms INTEGER,
            FOREIGN KEY (artist_stable_id) REFERENCES artists(stable_id),
            UNIQUE(artist_stable_id, album_id)
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

    // drops テーブル（期限付きファイル配信）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS drops (
            drop_id TEXT PRIMARY KEY,
            vendor_stable_id TEXT NOT NULL,
            artist_stable_id TEXT,
            artist_name TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            cover_object_key TEXT,
            audio_object_key TEXT NOT NULL,
            audio_mime TEXT NOT NULL DEFAULT 'audio/mpeg',
            audio_size_bytes INTEGER NOT NULL DEFAULT 0,
            audio_sha256 TEXT NOT NULL,
            start_at INTEGER NOT NULL,
            end_at INTEGER NOT NULL,
            max_claims INTEGER NOT NULL,
            claimed_count INTEGER NOT NULL DEFAULT 0,
            status INTEGER NOT NULL DEFAULT 0,
            env TEXT NOT NULL DEFAULT 'devnet',
            run_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            ended_at INTEGER,
            purged_at INTEGER,
            FOREIGN KEY (vendor_stable_id) REFERENCES vendors(stable_id)
        )
    "#)
    .execute(pool)
    .await?;

    // drop_claims テーブル（先着管理）
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS drop_claims (
            claim_id TEXT PRIMARY KEY,
            drop_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            device_id_hash TEXT,
            claimed_at INTEGER NOT NULL,
            FOREIGN KEY (drop_id) REFERENCES drops(drop_id),
            UNIQUE(drop_id, user_id)
        )
    "#)
    .execute(pool)
    .await?;

    // インデックス作成
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_vendors_is_alive ON vendors(is_alive)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_artists_is_alive ON artists(is_alive)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_discography_artist ON discography(artist_stable_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_listings_vendor ON listings(vendor_stable_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_listings_is_alive ON listings(is_alive)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_receipts_buyer ON receipts(buyer)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_receipts_listing ON receipts(listing_id)")
        .execute(pool).await?;

    // drops インデックス
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_drops_vendor ON drops(vendor_stable_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_drops_status ON drops(status)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_drops_end_at ON drops(end_at)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_drop_claims_drop ON drop_claims(drop_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_drop_claims_user ON drop_claims(user_id)")
        .execute(pool).await?;

    Ok(())
}
