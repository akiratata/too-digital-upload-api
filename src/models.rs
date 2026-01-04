//! Data Models
//! Vendor, Listing, Receipt などのデータ構造定義

use serde::{Deserialize, Serialize};

// ========================================
// Vendor
// ========================================

/// Vendor (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Vendor {
    pub stable_id: String,
    pub peer_id: Option<String>,
    pub peer_id_sha256: Option<String>,
    pub latest_object_id: Option<String>,
    pub owner: Option<String>,
    pub mode: i32,
    pub shop_type: i32,  // 0=in_app, 1=external_web
    pub backend: i32,    // 0=VPS, 1=Sui
    pub manifest_url: Option<String>,
    pub manifest_sha256: Option<String>,
    pub profile_seq: i64,
    pub status: i32,
    pub env: String,
    pub run_id: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: i32,
}

/// Vendor Profile (manifest JSON の中身)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorProfile {
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub address: Option<String>,
    pub fee_rate: Option<f64>,
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// Vendor 作成リクエスト
#[derive(Debug, Deserialize)]
pub struct CreateVendorRequest {
    pub stable_id: Option<String>,  // 指定しない場合は自動生成
    pub peer_id: String,
    pub object_id: Option<String>,
    pub owner: Option<String>,
    #[serde(default)]
    pub mode: i32,
    #[serde(default)]
    pub shop_type: i32,  // 0=in_app, 1=external_web
    #[serde(default)]
    pub backend: i32,    // 0=VPS, 1=Sui
    pub profile: VendorProfile,
    #[serde(default = "default_env")]
    pub env: String,
}

/// Vendor 更新リクエスト
#[derive(Debug, Deserialize)]
pub struct UpdateVendorRequest {
    pub object_id: Option<String>,
    pub owner: Option<String>,
    pub profile: Option<VendorProfile>,
    pub status: Option<i32>,
}

/// Vendor レスポンス（API返却用）
#[derive(Debug, Serialize)]
pub struct VendorResponse {
    pub stable_id: String,
    pub peer_id: Option<String>,
    pub object_id: Option<String>,
    pub owner: Option<String>,
    pub mode: i32,
    pub shop_type: i32,
    pub backend: i32,    // 0=VPS, 1=Sui
    pub profile: Option<VendorProfile>,
    pub profile_seq: i64,
    pub status: i32,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: bool,
}

// ========================================
// Listing
// ========================================

/// Listing (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Listing {
    pub listing_id: String,
    pub vendor_stable_id: String,
    pub vendor_object_id: Option<String>,
    pub seller: Option<String>,
    pub item_type: i32,
    pub item_id: Option<String>,
    pub price: i64,
    pub currency: String,
    pub supply_total: i64,
    pub supply_remaining: i64,
    pub status: i32,
    pub env: String,
    pub run_id: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: i32,
    // メタデータフィールド
    pub manifest_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
}

/// Listing 作成リクエスト
#[derive(Debug, Deserialize)]
pub struct CreateListingRequest {
    pub listing_id: String,
    pub vendor_stable_id: String,
    pub vendor_object_id: Option<String>,
    pub seller: Option<String>,
    #[serde(default)]
    pub item_type: i32,
    pub item_id: Option<String>,
    pub price: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default = "default_supply")]
    pub supply_total: i64,
    // メタデータフィールド
    pub manifest_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
}

fn default_currency() -> String { "SUI".to_string() }
fn default_supply() -> i64 { 1 }

/// Listing 更新リクエスト
#[derive(Debug, Deserialize)]
pub struct UpdateListingRequest {
    pub seller: Option<String>,
    pub price: Option<i64>,
    pub supply_remaining: Option<i64>,
    pub status: Option<i32>,
}

/// Listing レスポンス（API返却用）
#[derive(Debug, Serialize)]
pub struct ListingResponse {
    pub listing_id: String,
    pub vendor_stable_id: String,
    pub vendor_object_id: Option<String>,
    pub seller: Option<String>,
    pub item_type: i32,
    pub item_id: Option<String>,
    pub price: i64,
    pub currency: String,
    pub supply_total: i64,
    pub supply_remaining: i64,
    pub status: i32,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: bool,
    // メタデータフィールド
    pub manifest_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
}

// ========================================
// Receipt
// ========================================

/// Receipt (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Receipt {
    pub receipt_id: String,
    pub vendor_stable_id: String,
    pub listing_id: String,
    pub buyer: String,
    pub qty: i64,
    pub price: i64,
    pub currency: String,
    pub timestamp_ms: i64,
    pub tx_digest: Option<String>,
    pub env: String,
    pub run_id: Option<String>,
}

/// Receipt 作成リクエスト
#[derive(Debug, Deserialize)]
pub struct CreateReceiptRequest {
    pub receipt_id: String,
    pub vendor_stable_id: String,
    pub listing_id: String,
    pub buyer: String,
    #[serde(default = "default_qty")]
    pub qty: i64,
    pub price: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub timestamp_ms: i64,
    pub tx_digest: Option<String>,
}

fn default_qty() -> i64 { 1 }

// ========================================
// Status Constants
// ========================================

pub mod status {
    pub const ACTIVE: i32 = 0;
    pub const SUSPENDED: i32 = 1;
    pub const DELETED: i32 = 2;
    pub const SOLD_OUT: i32 = 3;
    pub const CANCELLED: i32 = 4;
}

pub mod item_type {
    pub const NFT: i32 = 0;
    pub const FILE_DROP: i32 = 1;
    pub const EDITION: i32 = 2;
}

pub mod mode {
    pub const TEST_VENDOR: i32 = 0;
    pub const PROD_VENDOR: i32 = 1;
}

pub mod shop_type {
    pub const IN_APP: i32 = 0;
    pub const EXTERNAL_WEB: i32 = 1;
}

// ========================================
// Artist
// ========================================

/// Artist (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Artist {
    pub stable_id: String,
    pub peer_id: String,
    pub peer_id_sha256: Option<String>,
    pub latest_object_id: Option<String>,
    pub owner: Option<String>,
    pub profile_url: Option<String>,
    pub profile_sha256: Option<String>,
    pub discography_url: Option<String>,
    pub discography_sha256: Option<String>,
    pub profile_seq: i64,
    pub status: i32,
    pub env: String,
    pub run_id: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: i32,
}

/// Artist Profile (profile.json の中身)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistProfile {
    pub version: String,
    pub stable_id: String,
    pub name: String,
    pub bio: Option<String>,
    pub icon_url: Option<String>,
    #[serde(default)]
    pub links: Vec<serde_json::Value>,
    pub p2p: Option<ArtistP2P>,
    pub updated_at_ms: i64,
}

/// Artist P2P info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistP2P {
    pub peer_id: String,
    pub peer_id_sha256: Option<String>,
}

/// Artist 作成リクエスト
#[derive(Debug, Deserialize)]
pub struct CreateArtistRequest {
    pub peer_id: String,
    pub name: String,
    pub bio: Option<String>,
    pub owner: Option<String>,
    #[serde(default = "default_env")]
    pub env: String,
}

fn default_env() -> String { "devnet".to_string() }

/// Artist 更新リクエスト
#[derive(Debug, Deserialize)]
pub struct UpdateArtistRequest {
    pub object_id: Option<String>,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub status: Option<i32>,
}

/// Artist レスポンス（API返却用）
#[derive(Debug, Serialize)]
pub struct ArtistResponse {
    pub stable_id: String,
    pub peer_id: String,
    pub object_id: Option<String>,
    pub owner: Option<String>,
    pub profile: Option<ArtistProfile>,
    pub profile_url: Option<String>,
    pub profile_sha256: Option<String>,
    pub discography_url: Option<String>,
    pub discography_sha256: Option<String>,
    pub profile_seq: i64,
    pub status: i32,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub is_alive: bool,
}

/// Artist 作成レスポンス
#[derive(Debug, Serialize)]
pub struct ArtistCreateResponse {
    pub success: bool,
    pub stable_id: String,
    pub peer_id: String,
    pub profile_url: String,
    pub profile_sha256: String,
    pub discography_url: String,
    pub discography_sha256: String,
    pub icon_url: Option<String>,
    pub updated_at_ms: i64,
}

// ========================================
// Discography
// ========================================

/// Discography Entry (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DiscographyEntry {
    pub id: i64,
    pub artist_stable_id: String,
    pub album_id: String,
    pub edition_id: Option<String>,
    pub title: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub track_count: i64,
    pub track_preview: Option<String>,
    pub role: String,
    pub deployed_at_ms: Option<i64>,
    pub created_at_ms: Option<i64>,
}

/// Track Preview (discography.json 内の track_preview)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackPreview {
    pub i: i32,
    pub title: String,
}

/// Discography JSON (discography.json の中身)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscographyJson {
    pub version: String,
    pub artist_stable_id: String,
    pub albums: Vec<DiscographyAlbum>,
    pub updated_at_ms: i64,
}

/// Discography Album Entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscographyAlbum {
    pub album_id: String,
    pub edition_id: Option<String>,
    pub title: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub track_count: i64,
    pub track_preview: Vec<TrackPreview>,
    pub deployed_at_ms: Option<i64>,
    pub role: String,
}

/// Discography 追加リクエスト
#[derive(Debug, Deserialize)]
pub struct AddDiscographyRequest {
    pub album_id: String,
    pub edition_id: Option<String>,
    pub title: Option<String>,
    pub cover_thumb_url: Option<String>,
    #[serde(default)]
    pub track_count: i64,
    #[serde(default)]
    pub track_preview: Vec<TrackPreview>,
    #[serde(default = "default_role")]
    pub role: String,
    pub deployed_at_ms: Option<i64>,
}

fn default_role() -> String { "main".to_string() }

// ========================================
// Drop (期限付きファイル配信)
// ========================================

/// Drop ステータス
pub mod drop_status {
    pub const SCHEDULED: i32 = 0;
    pub const ACTIVE: i32 = 1;
    pub const ENDED: i32 = 2;
    pub const PURGED: i32 = 3;
}

/// Drop (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Drop {
    pub drop_id: String,
    pub vendor_stable_id: String,
    pub artist_stable_id: Option<String>,
    pub artist_name: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_object_key: Option<String>,
    pub audio_object_key: String,
    pub audio_mime: String,
    pub audio_size_bytes: i64,
    pub audio_sha256: String,
    pub start_at: i64,      // Unix秒
    pub end_at: i64,        // Unix秒
    pub max_claims: i64,
    pub claimed_count: i64,
    pub status: i32,
    pub env: String,
    pub run_id: Option<String>,
    pub created_at: i64,    // Unix秒
    pub updated_at: i64,    // Unix秒
    pub ended_at: Option<i64>,   // Unix秒
    pub purged_at: Option<i64>,  // Unix秒
}

/// Drop 作成リクエスト
#[derive(Debug, Deserialize)]
pub struct CreateDropRequest {
    pub vendor_stable_id: String,
    pub artist_stable_id: Option<String>,
    pub artist_name: String,
    pub title: String,
    pub description: Option<String>,
    pub start_at: Option<i64>,  // 省略時は現在時刻
    pub end_at: i64,            // 必須
    pub max_claims: i64,        // 必須
    #[serde(default = "default_env")]
    pub env: String,
}

/// Drop レスポンス
#[derive(Debug, Serialize)]
pub struct DropResponse {
    pub drop_id: String,
    pub vendor_stable_id: String,
    pub artist_stable_id: Option<String>,
    pub artist_name: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub audio_mime: String,
    pub audio_size_bytes: i64,
    pub audio_sha256: String,
    pub start_at: i64,
    pub end_at: i64,
    pub max_claims: i64,
    pub claimed_count: i64,
    pub remaining_claims: i64,
    pub status: i32,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
}

impl DropResponse {
    pub fn from_drop(drop: &Drop, base_url: &str) -> Self {
        // カバーURLとサムネイルURLを生成
        // cover_object_key: "DROP_XXX/cover.jpg" → URL: "{base_url}/drops/DROP_XXX/cover.jpg"
        let cover_url = drop.cover_object_key.as_ref().map(|key| {
            format!("{}/drops/{}", base_url, key)
        });
        // サムネイル: cover.jpg → cover_thumb.jpg
        let cover_thumb_url = drop.cover_object_key.as_ref().map(|key| {
            // "DROP_XXX/cover.jpg" → "DROP_XXX/cover_thumb.jpg"
            if let Some(dot_pos) = key.rfind('.') {
                let (base, ext) = key.split_at(dot_pos);
                format!("{}/drops/{}_thumb{}", base_url, base, ext)
            } else {
                format!("{}/drops/{}_thumb", base_url, key)
            }
        });
        Self {
            drop_id: drop.drop_id.clone(),
            vendor_stable_id: drop.vendor_stable_id.clone(),
            artist_stable_id: drop.artist_stable_id.clone(),
            artist_name: drop.artist_name.clone(),
            title: drop.title.clone(),
            description: drop.description.clone(),
            cover_url,
            cover_thumb_url,
            audio_mime: drop.audio_mime.clone(),
            audio_size_bytes: drop.audio_size_bytes,
            audio_sha256: drop.audio_sha256.clone(),
            start_at: drop.start_at,
            end_at: drop.end_at,
            max_claims: drop.max_claims,
            claimed_count: drop.claimed_count,
            remaining_claims: drop.max_claims - drop.claimed_count,
            status: drop.status,
            created_at: drop.created_at,
            updated_at: drop.updated_at,
            ended_at: drop.ended_at,
        }
    }
}

/// Drop Claim (DB row)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DropClaim {
    pub claim_id: String,
    pub drop_id: String,
    pub user_id: String,
    pub device_id_hash: Option<String>,
    pub claimed_at: i64,    // Unix秒
}

/// Drop Claim リクエスト
#[derive(Debug, Deserialize)]
pub struct ClaimDropRequest {
    pub user_id: String,
    pub device_id_hash: Option<String>,
}

/// Drop Claim レスポンス
#[derive(Debug, Serialize)]
pub struct ClaimDropResponse {
    pub success: bool,
    pub claim_id: String,
    pub drop_id: String,
    pub download_url: String,
    pub expires_at: i64,
    pub audio_sha256: String,
    pub audio_size_bytes: i64,
}

/// Batch 終了/削除リクエスト
#[derive(Debug, Deserialize)]
pub struct BatchDropRequest {
    pub drop_ids: Vec<String>,
}

/// Batch レスポンス
#[derive(Debug, Serialize)]
pub struct BatchDropResponse {
    pub success: bool,
    pub results: std::collections::HashMap<String, bool>,
}
