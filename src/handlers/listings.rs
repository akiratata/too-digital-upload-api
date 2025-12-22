//! Listings API Handlers
//! /api/listings エンドポイント

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::models::{
    CreateListingRequest, Listing, ListingResponse, UpdateListingRequest,
};
use crate::AppState;

// ========================================
// Response Types
// ========================================

#[derive(Serialize)]
pub struct ListingListResponse {
    pub success: bool,
    pub listings: Vec<ListingResponse>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct ListingDetailResponse {
    pub success: bool,
    pub listing: Option<ListingResponse>,
}

#[derive(Serialize)]
pub struct ListingCreateResponse {
    pub success: bool,
    pub listing_id: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
}

// ========================================
// Query Parameters
// ========================================

#[derive(Debug, Deserialize)]
pub struct ListListingsQuery {
    pub vendor_stable_id: Option<String>,
    pub status: Option<i32>,
}

// ========================================
// Handlers
// ========================================

/// GET /api/listings - Listing一覧取得
pub async fn list_listings(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListListingsQuery>,
) -> Result<Json<ListingListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let listings: Vec<Listing> = if let Some(vendor_id) = &query.vendor_stable_id {
        sqlx::query_as(
            "SELECT * FROM listings WHERE vendor_stable_id = ? AND is_alive = 1 ORDER BY created_at_ms DESC"
        )
        .bind(vendor_id)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as(
            "SELECT * FROM listings WHERE is_alive = 1 ORDER BY created_at_ms DESC"
        )
        .fetch_all(&state.db)
        .await
    }
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    let responses: Vec<ListingResponse> = listings
        .iter()
        .filter(|l| query.status.map_or(true, |s| l.status == s))
        .map(listing_to_response)
        .collect();

    let total = responses.len();
    Ok(Json(ListingListResponse {
        success: true,
        listings: responses,
        total,
    }))
}

/// GET /api/listings/:listing_id - Listing詳細取得
pub async fn get_listing(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
) -> Result<Json<ListingDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let listing: Option<Listing> = sqlx::query_as(
        "SELECT * FROM listings WHERE listing_id = ?"
    )
    .bind(&listing_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    match listing {
        Some(l) => Ok(Json(ListingDetailResponse {
            success: true,
            listing: Some(listing_to_response(&l)),
        })),
        None => Err(error_response(StatusCode::NOT_FOUND, "Listing not found".to_string())),
    }
}

/// POST /api/listings - Listing作成
pub async fn create_listing(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateListingRequest>,
) -> Result<Json<ListingCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // Vendor存在チェック
    let vendor_exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM vendors WHERE stable_id = ? AND is_alive = 1"
    )
    .bind(&req.vendor_stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if vendor_exists.is_none() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Vendor not found: {}", req.vendor_stable_id),
        ));
    }

    // DBに挿入
    sqlx::query(r#"
        INSERT INTO listings (
            listing_id, vendor_stable_id, vendor_object_id, seller,
            item_type, item_id, price, currency,
            supply_total, supply_remaining, status,
            env, created_at_ms, updated_at_ms, is_alive,
            manifest_id, title, artist, cover_url
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 'devnet', ?, ?, 1, ?, ?, ?, ?)
        ON CONFLICT(listing_id) DO UPDATE SET
            vendor_object_id = COALESCE(excluded.vendor_object_id, listings.vendor_object_id),
            seller = COALESCE(excluded.seller, listings.seller),
            price = excluded.price,
            supply_remaining = excluded.supply_remaining,
            updated_at_ms = excluded.updated_at_ms,
            is_alive = 1,
            manifest_id = COALESCE(excluded.manifest_id, listings.manifest_id),
            title = COALESCE(excluded.title, listings.title),
            artist = COALESCE(excluded.artist, listings.artist),
            cover_url = COALESCE(excluded.cover_url, listings.cover_url)
    "#)
    .bind(&req.listing_id)
    .bind(&req.vendor_stable_id)
    .bind(&req.vendor_object_id)
    .bind(&req.seller)
    .bind(req.item_type)
    .bind(&req.item_id)
    .bind(req.price)
    .bind(&req.currency)
    .bind(req.supply_total)
    .bind(req.supply_total) // supply_remaining = supply_total initially
    .bind(now_ms)
    .bind(now_ms)
    .bind(&req.manifest_id)
    .bind(&req.title)
    .bind(&req.artist)
    .bind(&req.cover_url)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    info!("Listing created: listing_id={}, vendor={}", req.listing_id, req.vendor_stable_id);

    Ok(Json(ListingCreateResponse {
        success: true,
        listing_id: req.listing_id,
    }))
}

/// PUT /api/listings/:listing_id - Listing更新
pub async fn update_listing(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    Json(req): Json<UpdateListingRequest>,
) -> Result<Json<ListingCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 既存チェック
    let existing: Option<Listing> = sqlx::query_as(
        "SELECT * FROM listings WHERE listing_id = ?"
    )
    .bind(&listing_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if existing.is_none() {
        return Err(error_response(StatusCode::NOT_FOUND, "Listing not found".to_string()));
    }

    // DB更新
    sqlx::query(r#"
        UPDATE listings SET
            seller = COALESCE(?, seller),
            price = COALESCE(?, price),
            supply_remaining = COALESCE(?, supply_remaining),
            status = COALESCE(?, status),
            updated_at_ms = ?
        WHERE listing_id = ?
    "#)
    .bind(&req.seller)
    .bind(req.price)
    .bind(req.supply_remaining)
    .bind(req.status)
    .bind(now_ms)
    .bind(&listing_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    info!("Listing updated: listing_id={}", listing_id);

    Ok(Json(ListingCreateResponse {
        success: true,
        listing_id,
    }))
}

/// DELETE /api/listings/:listing_id - Listing削除（論理削除）
pub async fn delete_listing(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
) -> Result<Json<ListingCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    let result = sqlx::query(
        "UPDATE listings SET is_alive = 0, updated_at_ms = ? WHERE listing_id = ?"
    )
    .bind(now_ms)
    .bind(&listing_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if result.rows_affected() == 0 {
        return Err(error_response(StatusCode::NOT_FOUND, "Listing not found".to_string()));
    }

    info!("Listing deleted: listing_id={}", listing_id);

    Ok(Json(ListingCreateResponse {
        success: true,
        listing_id,
    }))
}

// ========================================
// Helper Functions
// ========================================

fn listing_to_response(l: &Listing) -> ListingResponse {
    ListingResponse {
        listing_id: l.listing_id.clone(),
        vendor_stable_id: l.vendor_stable_id.clone(),
        vendor_object_id: l.vendor_object_id.clone(),
        seller: l.seller.clone(),
        item_type: l.item_type,
        item_id: l.item_id.clone(),
        price: l.price,
        currency: l.currency.clone(),
        supply_total: l.supply_total,
        supply_remaining: l.supply_remaining,
        status: l.status,
        created_at_ms: l.created_at_ms,
        updated_at_ms: l.updated_at_ms,
        is_alive: l.is_alive == 1,
        manifest_id: l.manifest_id.clone(),
        title: l.title.clone(),
        artist: l.artist.clone(),
        cover_url: l.cover_url.clone(),
    }
}

fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    warn!("API Error: {}", message);
    (status, Json(ErrorResponse { success: false, error: message }))
}
