//! Vendors API Handlers
//! /api/vendors エンドポイント

use axum::{
    extract::{Path, State, Multipart},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use sha2::{Sha256, Digest};
use base32;
use rand::Rng;

use crate::models::{
    CreateVendorRequest, UpdateVendorRequest, Vendor, VendorProfile, VendorResponse,
};
use crate::AppState;

// ========================================
// Response Types
// ========================================

#[derive(Serialize)]
pub struct VendorListResponse {
    pub success: bool,
    pub vendors: Vec<VendorResponse>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct VendorDetailResponse {
    pub success: bool,
    pub vendor: Option<VendorResponse>,
}

#[derive(Serialize)]
pub struct VendorCreateResponse {
    pub success: bool,
    pub stable_id: String,
    pub peer_id: String,
    pub manifest_url: String,
    pub manifest_sha256: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
}

// ========================================
// Handlers
// ========================================

/// GET /api/vendors - Vendor一覧取得
pub async fn list_vendors(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VendorListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let vendors: Vec<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE is_alive = 1 ORDER BY created_at_ms DESC"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    let mut responses = Vec::new();
    for v in &vendors {
        let profile = load_vendor_profile(&state.base_data_dir, &v.stable_id).await.ok();
        responses.push(vendor_to_response(v, profile));
    }

    let total = responses.len();
    Ok(Json(VendorListResponse {
        success: true,
        vendors: responses,
        total,
    }))
}

/// GET /api/vendors/:stable_id - Vendor詳細取得
pub async fn get_vendor(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
) -> Result<Json<VendorDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let vendor: Option<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    match vendor {
        Some(v) => {
            let profile = load_vendor_profile(&state.base_data_dir, &v.stable_id).await.ok();
            Ok(Json(VendorDetailResponse {
                success: true,
                vendor: Some(vendor_to_response(&v, profile)),
            }))
        }
        None => Err(error_response(StatusCode::NOT_FOUND, "Vendor not found".to_string())),
    }
}

/// GET /api/vendors/by-peer/:peer_id - peer_idでVendor検索
pub async fn get_vendor_by_peer(
    State(state): State<Arc<AppState>>,
    Path(peer_id): Path<String>,
) -> Result<Json<VendorDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let vendor: Option<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE peer_id = ? AND is_alive = 1"
    )
    .bind(&peer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    match vendor {
        Some(v) => {
            let profile = load_vendor_profile(&state.base_data_dir, &v.stable_id).await.ok();
            Ok(Json(VendorDetailResponse {
                success: true,
                vendor: Some(vendor_to_response(&v, profile)),
            }))
        }
        None => Err(error_response(StatusCode::NOT_FOUND, "Vendor not found for this peer_id".to_string())),
    }
}

/// POST /api/vendors - Vendor作成
pub async fn create_vendor(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateVendorRequest>,
) -> Result<Json<VendorCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 既存チェック（peer_idで、is_alive=1のみ）
    let existing: Option<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE peer_id = ? AND is_alive = 1"
    )
    .bind(&req.peer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if let Some(v) = existing {
        // 既存のアクティブなVendorを返す
        return Ok(Json(VendorCreateResponse {
            success: true,
            stable_id: v.stable_id,
            peer_id: req.peer_id,
            manifest_url: v.manifest_url.unwrap_or_default(),
            manifest_sha256: v.manifest_sha256.unwrap_or_default(),
        }));
    }

    // stable_id を生成（VENDOR_XXXXXXXX形式）
    let stable_id = req.stable_id.unwrap_or_else(|| generate_stable_id("VENDOR"));

    // peer_id の SHA256
    let peer_id_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(req.peer_id.as_bytes());
        hex::encode(hasher.finalize())
    };

    // ディレクトリ作成
    let vendor_dir = PathBuf::from(&state.base_data_dir)
        .join("vendors")
        .join(&stable_id);
    fs::create_dir_all(&vendor_dir).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {}", e))
    })?;

    // profile.json を保存
    let (manifest_url, manifest_sha256) = save_vendor_profile(
        &state.base_data_dir,
        &state.vps_base_url,
        &stable_id,
        &req.profile,
    )
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save profile: {}", e))
    })?;

    // DBに挿入
    sqlx::query(r#"
        INSERT INTO vendors (
            stable_id, peer_id, peer_id_sha256, latest_object_id, owner, mode, shop_type,
            manifest_url, manifest_sha256, profile_seq,
            status, env, created_at_ms, updated_at_ms, is_alive
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 0, ?, ?, ?, 1)
    "#)
    .bind(&stable_id)
    .bind(&req.peer_id)
    .bind(&peer_id_sha256)
    .bind(&req.object_id)
    .bind(&req.owner)
    .bind(req.mode)
    .bind(req.shop_type)
    .bind(&manifest_url)
    .bind(&manifest_sha256)
    .bind(&req.env)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    info!("Vendor created: stable_id={}, peer_id={}", stable_id, req.peer_id);

    Ok(Json(VendorCreateResponse {
        success: true,
        stable_id,
        peer_id: req.peer_id,
        manifest_url,
        manifest_sha256,
    }))
}

/// PUT /api/vendors/:stable_id - Vendor更新
pub async fn update_vendor(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
    Json(req): Json<UpdateVendorRequest>,
) -> Result<Json<VendorCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 既存チェック
    let existing: Option<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    let v = match existing {
        Some(v) => v,
        None => return Err(error_response(StatusCode::NOT_FOUND, "Vendor not found".to_string())),
    };

    let (manifest_url, manifest_sha256) = if let Some(profile) = &req.profile {
        save_vendor_profile(
            &state.base_data_dir,
            &state.vps_base_url,
            &stable_id,
            profile,
        )
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save profile: {}", e))
        })?
    } else {
        (v.manifest_url.clone().unwrap_or_default(), v.manifest_sha256.clone().unwrap_or_default())
    };

    // DB更新
    sqlx::query(r#"
        UPDATE vendors SET
            latest_object_id = COALESCE(?, latest_object_id),
            owner = COALESCE(?, owner),
            manifest_url = ?,
            manifest_sha256 = ?,
            profile_seq = profile_seq + 1,
            status = COALESCE(?, status),
            updated_at_ms = ?
        WHERE stable_id = ?
    "#)
    .bind(&req.object_id)
    .bind(&req.owner)
    .bind(&manifest_url)
    .bind(&manifest_sha256)
    .bind(req.status)
    .bind(now_ms)
    .bind(&stable_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    info!("Vendor updated: stable_id={}", stable_id);

    Ok(Json(VendorCreateResponse {
        success: true,
        stable_id,
        peer_id: v.peer_id.unwrap_or_default(),
        manifest_url,
        manifest_sha256,
    }))
}

/// DELETE /api/vendors/:stable_id - Vendorをデリスト（論理削除）
pub async fn delist_vendor(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 既存チェック
    let existing: Option<Vendor> = sqlx::query_as(
        "SELECT * FROM vendors WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    match existing {
        Some(v) => {
            // is_alive を 0 に設定（論理削除）
            sqlx::query(
                "UPDATE vendors SET is_alive = 0, updated_at_ms = ? WHERE stable_id = ?"
            )
            .bind(now_ms)
            .bind(&stable_id)
            .execute(&state.db)
            .await
            .map_err(|e| {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
            })?;

            info!("Vendor delisted: stable_id={}, peer_id={:?}", stable_id, v.peer_id);

            Ok(Json(serde_json::json!({
                "success": true,
                "stable_id": stable_id,
                "message": "Vendor delisted successfully"
            })))
        }
        None => Err(error_response(StatusCode::NOT_FOUND, "Vendor not found".to_string())),
    }
}

/// POST /api/vendors/:stable_id/icon - アイコンアップロード
pub async fn upload_vendor_icon(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // ファイルを取得
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error_response(StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "icon" {
            let filename = field.file_name().unwrap_or("icon.webp").to_string();
            let ext = filename.split('.').last().unwrap_or("webp");

            let data = field.bytes().await.map_err(|e| {
                error_response(StatusCode::BAD_REQUEST, format!("File read error: {}", e))
            })?;

            // 保存先ディレクトリ
            let dir = PathBuf::from(&state.base_data_dir)
                .join("vendors")
                .join(&stable_id);
            fs::create_dir_all(&dir).await.map_err(|e| {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {}", e))
            })?;

            // ファイル保存
            let icon_filename = format!("icon.{}", ext);
            let path = dir.join(&icon_filename);
            let mut file = fs::File::create(&path).await.map_err(|e| {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create file: {}", e))
            })?;
            file.write_all(&data).await.map_err(|e| {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write file: {}", e))
            })?;

            let icon_url = format!("{}/vendors/{}/{}", state.vps_base_url, stable_id, icon_filename);
            info!("Icon uploaded: {}", icon_url);

            // profile.json の icon_url を更新
            let profile_path = dir.join("profile.json");
            if profile_path.exists() {
                if let Ok(content) = fs::read_to_string(&profile_path).await {
                    if let Ok(mut profile) = serde_json::from_str::<VendorProfile>(&content) {
                        profile.icon_url = Some(icon_url.clone());
                        if let Ok(updated_json) = serde_json::to_string_pretty(&profile) {
                            let _ = fs::write(&profile_path, updated_json).await;
                            info!("Profile updated with icon_url: {}", icon_url);
                        }
                    }
                }
            }

            return Ok(Json(serde_json::json!({
                "success": true,
                "icon_url": icon_url,
                "path": path.to_string_lossy()
            })));
        }
    }

    Err(error_response(StatusCode::BAD_REQUEST, "No file provided".to_string()))
}

// ========================================
// Helper Functions
// ========================================

/// stable_id を生成（PREFIX_XXXXXXXX形式）
fn generate_stable_id(prefix: &str) -> String {
    let random_bytes: [u8; 5] = rand::thread_rng().gen();
    let encoded = base32::encode(base32::Alphabet::Crockford, &random_bytes);
    format!("{}_{}", prefix, &encoded[..8])
}

/// VendorProfile を保存して URL と SHA256 を返す
async fn save_vendor_profile(
    base_dir: &str,
    base_url: &str,
    stable_id: &str,
    profile: &VendorProfile,
) -> anyhow::Result<(String, String)> {
    let dir = PathBuf::from(base_dir)
        .join("vendors")
        .join(stable_id);
    fs::create_dir_all(&dir).await?;

    let json = serde_json::to_string_pretty(profile)?;

    // SHA256 計算
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let sha256 = hex::encode(hasher.finalize());

    // ファイル保存
    let path = dir.join("profile.json");
    let mut file = fs::File::create(&path).await?;
    file.write_all(json.as_bytes()).await?;

    let url = format!("{}/vendors/{}/profile.json", base_url, stable_id);

    info!("Profile saved: {} (sha256: {})", url, &sha256[..16]);

    Ok((url, sha256))
}

/// VendorProfile をファイルから読み込む
async fn load_vendor_profile(base_dir: &str, stable_id: &str) -> anyhow::Result<VendorProfile> {
    let path = PathBuf::from(base_dir)
        .join("vendors")
        .join(stable_id)
        .join("profile.json");

    let content = fs::read_to_string(&path).await?;
    let profile: VendorProfile = serde_json::from_str(&content)?;
    Ok(profile)
}

/// Vendor を VendorResponse に変換
fn vendor_to_response(v: &Vendor, profile: Option<VendorProfile>) -> VendorResponse {
    VendorResponse {
        stable_id: v.stable_id.clone(),
        peer_id: v.peer_id.clone(),
        object_id: v.latest_object_id.clone(),
        owner: v.owner.clone(),
        mode: v.mode,
        shop_type: v.shop_type,
        profile,
        profile_seq: v.profile_seq,
        status: v.status,
        created_at_ms: v.created_at_ms,
        updated_at_ms: v.updated_at_ms,
        is_alive: v.is_alive == 1,
    }
}

/// エラーレスポンス生成
fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    warn!("API Error: {}", message);
    (status, Json(ErrorResponse { success: false, error: message }))
}
