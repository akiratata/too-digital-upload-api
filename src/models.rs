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
    pub latest_object_id: Option<String>,
    pub owner: Option<String>,
    pub mode: i32,
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
    pub stable_id: String,
    pub object_id: Option<String>,
    pub owner: Option<String>,
    #[serde(default)]
    pub mode: i32,
    pub profile: VendorProfile,
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
    pub object_id: Option<String>,
    pub owner: Option<String>,
    pub mode: i32,
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
}

fn default_currency() -> String { "SUI".to_string() }
fn default_supply() -> i64 { 1 }

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
