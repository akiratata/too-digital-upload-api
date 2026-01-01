//! Drops API Handlers
//! /api/drops エンドポイント - 期限付きファイル配信

use axum::{
    extract::{Path, Query, State, Multipart},
    http::StatusCode,
    response::Json,
    body::Body,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use sha2::{Sha256, Digest};
use base32;
use rand::Rng;
use uuid::Uuid;

use crate::models::{
    Drop, DropResponse, DropClaim, ClaimDropRequest, ClaimDropResponse,
    BatchDropRequest, BatchDropResponse, drop_status,
};
use crate::AppState;

// ========================================
// Response Types
// ========================================

#[derive(Serialize)]
pub struct DropListResponse {
    pub success: bool,
    pub drops: Vec<DropResponse>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct DropDetailResponse {
    pub success: bool,
    pub drop: Option<DropResponse>,
}

#[derive(Serialize)]
pub struct DropCreateResponse {
    pub success: bool,
    pub drop: DropResponse,
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
pub struct ListDropsQuery {
    pub status: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub token: Option<String>,
}

// ========================================
// Handlers
// ========================================

/// GET /api/vendors/:vendor_stable_id/drops - Vendor別Drop一覧
pub async fn list_drops(
    State(state): State<Arc<AppState>>,
    Path(vendor_stable_id): Path<String>,
    Query(query): Query<ListDropsQuery>,
) -> Result<Json<DropListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = chrono::Utc::now().timestamp();

    // 期限切れのDropをENDEDに更新（クエリ時に自動処理）
    let _ = sqlx::query(
        "UPDATE drops SET status = ?, ended_at = ? WHERE end_at <= ? AND status IN (?, ?)"
    )
    .bind(drop_status::ENDED)
    .bind(now)
    .bind(now)
    .bind(drop_status::SCHEDULED)
    .bind(drop_status::ACTIVE)
    .execute(&state.db)
    .await;

    let drops: Vec<Drop> = if let Some(status) = query.status {
        sqlx::query_as(
            "SELECT * FROM drops WHERE vendor_stable_id = ? AND status = ? ORDER BY created_at DESC"
        )
        .bind(&vendor_stable_id)
        .bind(status)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as(
            "SELECT * FROM drops WHERE vendor_stable_id = ? AND status != ? ORDER BY created_at DESC"
        )
        .bind(&vendor_stable_id)
        .bind(drop_status::PURGED)
        .fetch_all(&state.db)
        .await
    }
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    let responses: Vec<DropResponse> = drops
        .iter()
        .map(|d| DropResponse::from_drop(d, &state.vps_base_url))
        .collect();

    let total = responses.len();
    Ok(Json(DropListResponse {
        success: true,
        drops: responses,
        total,
    }))
}

/// GET /api/drops/:drop_id - Drop詳細
pub async fn get_drop(
    State(state): State<Arc<AppState>>,
    Path(drop_id): Path<String>,
) -> Result<Json<DropDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let drop: Option<Drop> = sqlx::query_as(
        "SELECT * FROM drops WHERE drop_id = ?"
    )
    .bind(&drop_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    match drop {
        Some(d) => Ok(Json(DropDetailResponse {
            success: true,
            drop: Some(DropResponse::from_drop(&d, &state.vps_base_url)),
        })),
        None => Err(error_response(StatusCode::NOT_FOUND, "Drop not found".to_string())),
    }
}

/// POST /api/drops - Drop作成（Multipart）
pub async fn create_drop(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<DropCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = chrono::Utc::now().timestamp();
    let drop_id = generate_drop_id();

    // フォームデータを収集
    let mut vendor_stable_id: Option<String> = None;
    let mut artist_stable_id: Option<String> = None;
    let mut artist_name: Option<String> = None;
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut start_at: Option<i64> = None;
    let mut end_at: Option<i64> = None;
    let mut max_claims: Option<i64> = None;
    let mut env = "devnet".to_string();

    let mut audio_data: Option<Vec<u8>> = None;
    let mut audio_filename: Option<String> = None;
    let mut audio_mime: Option<String> = None;
    let mut cover_data: Option<Vec<u8>> = None;
    let mut cover_filename: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error_response(StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "vendor_stable_id" => {
                vendor_stable_id = Some(field.text().await.unwrap_or_default());
            }
            "artist_stable_id" => {
                let val = field.text().await.unwrap_or_default();
                if !val.is_empty() {
                    artist_stable_id = Some(val);
                }
            }
            "artist_name" => {
                artist_name = Some(field.text().await.unwrap_or_default());
            }
            "title" => {
                title = Some(field.text().await.unwrap_or_default());
            }
            "description" => {
                let val = field.text().await.unwrap_or_default();
                if !val.is_empty() {
                    description = Some(val);
                }
            }
            "start_at" => {
                if let Ok(val) = field.text().await.unwrap_or_default().parse::<i64>() {
                    start_at = Some(val);
                }
            }
            "end_at" => {
                if let Ok(val) = field.text().await.unwrap_or_default().parse::<i64>() {
                    end_at = Some(val);
                }
            }
            "max_claims" => {
                if let Ok(val) = field.text().await.unwrap_or_default().parse::<i64>() {
                    max_claims = Some(val);
                }
            }
            "env" => {
                env = field.text().await.unwrap_or_default();
            }
            "audio" => {
                audio_filename = field.file_name().map(|s| s.to_string());
                audio_mime = field.content_type().map(|s| s.to_string());
                audio_data = Some(field.bytes().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("Audio read error: {}", e))
                })?.to_vec());
            }
            "cover" => {
                cover_filename = field.file_name().map(|s| s.to_string());
                cover_data = Some(field.bytes().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("Cover read error: {}", e))
                })?.to_vec());
            }
            _ => {}
        }
    }

    // 必須フィールドチェック
    let vendor_stable_id = vendor_stable_id.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "vendor_stable_id is required".to_string())
    })?;
    let artist_name = artist_name.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "artist_name is required".to_string())
    })?;
    let title = title.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "title is required".to_string())
    })?;
    let end_at = end_at.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "end_at is required".to_string())
    })?;
    let max_claims = max_claims.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "max_claims is required".to_string())
    })?;
    let audio_data = audio_data.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "audio file is required".to_string())
    })?;

    // Vendor存在チェック
    let vendor_exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM vendors WHERE stable_id = ? AND is_alive = 1"
    )
    .bind(&vendor_stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if vendor_exists.is_none() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Vendor not found: {}", vendor_stable_id),
        ));
    }

    // ディレクトリ作成
    let dir = PathBuf::from(&state.base_data_dir)
        .join("drops")
        .join(&drop_id);
    fs::create_dir_all(&dir).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {}", e))
    })?;

    // 音声ファイル保存
    let audio_ext = audio_filename
        .as_ref()
        .and_then(|f| f.split('.').last())
        .unwrap_or("mp3");
    let audio_object_key = format!("{}/audio.{}", drop_id, audio_ext);
    let audio_path = dir.join(format!("audio.{}", audio_ext));
    let mut file = fs::File::create(&audio_path).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create audio file: {}", e))
    })?;
    file.write_all(&audio_data).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write audio: {}", e))
    })?;

    // SHA256計算
    let audio_sha256 = compute_sha256(&audio_data);
    let audio_size_bytes = audio_data.len() as i64;
    let audio_mime = audio_mime.unwrap_or_else(|| {
        // 拡張子からMIMEタイプを推測
        match audio_ext {
            "flac" => "audio/flac".to_string(),
            "wav" => "audio/wav".to_string(),
            "ogg" => "audio/ogg".to_string(),
            "aac" => "audio/aac".to_string(),
            "m4a" => "audio/mp4".to_string(),
            _ => "audio/mpeg".to_string(),
        }
    });

    // カバー画像保存（任意）
    let cover_object_key = if let Some(cover) = cover_data {
        let cover_ext = cover_filename
            .as_ref()
            .and_then(|f| f.split('.').last())
            .unwrap_or("jpg");
        let key = format!("{}/cover.{}", drop_id, cover_ext);
        let cover_path = dir.join(format!("cover.{}", cover_ext));
        let mut file = fs::File::create(&cover_path).await.map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create cover file: {}", e))
        })?;
        file.write_all(&cover).await.map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write cover: {}", e))
        })?;
        Some(key)
    } else {
        None
    };

    // start_at デフォルト設定
    let start_at = start_at.unwrap_or(now);
    let status = if now >= start_at { drop_status::ACTIVE } else { drop_status::SCHEDULED };

    // DB挿入
    sqlx::query(r#"
        INSERT INTO drops (
            drop_id, vendor_stable_id, artist_stable_id, artist_name,
            title, description, cover_object_key, audio_object_key,
            audio_mime, audio_size_bytes, audio_sha256,
            start_at, end_at, max_claims, claimed_count,
            status, env, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)
    "#)
    .bind(&drop_id)
    .bind(&vendor_stable_id)
    .bind(&artist_stable_id)
    .bind(&artist_name)
    .bind(&title)
    .bind(&description)
    .bind(&cover_object_key)
    .bind(&audio_object_key)
    .bind(&audio_mime)
    .bind(audio_size_bytes)
    .bind(&audio_sha256)
    .bind(start_at)
    .bind(end_at)
    .bind(max_claims)
    .bind(status)
    .bind(&env)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    info!("Drop created: drop_id={}, vendor={}, title={}", drop_id, vendor_stable_id, title);

    // レスポンス用にDropを取得
    let drop: Drop = sqlx::query_as("SELECT * FROM drops WHERE drop_id = ?")
        .bind(&drop_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
        })?;

    Ok(Json(DropCreateResponse {
        success: true,
        drop: DropResponse::from_drop(&drop, &state.vps_base_url),
    }))
}

/// POST /api/drops/:drop_id/claim - Drop受け取り
pub async fn claim_drop(
    State(state): State<Arc<AppState>>,
    Path(drop_id): Path<String>,
    Json(req): Json<ClaimDropRequest>,
) -> Result<Json<ClaimDropResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = chrono::Utc::now().timestamp();

    // Drop取得
    let drop: Option<Drop> = sqlx::query_as("SELECT * FROM drops WHERE drop_id = ?")
        .bind(&drop_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
        })?;

    let drop = drop.ok_or_else(|| {
        error_response(StatusCode::NOT_FOUND, "Drop not found".to_string())
    })?;

    // ステータスチェック
    if drop.status == drop_status::ENDED || drop.status == drop_status::PURGED {
        return Err(error_response(StatusCode::BAD_REQUEST, "Drop has ended".to_string()));
    }

    // 期限チェック
    if now < drop.start_at {
        return Err(error_response(StatusCode::BAD_REQUEST, "Drop has not started yet".to_string()));
    }
    if now >= drop.end_at {
        return Err(error_response(StatusCode::BAD_REQUEST, "Drop has expired".to_string()));
    }

    // 在庫チェック
    if drop.claimed_count >= drop.max_claims {
        return Err(error_response(StatusCode::BAD_REQUEST, "No more claims available".to_string()));
    }

    // 重複チェック
    let existing_claim: Option<DropClaim> = sqlx::query_as(
        "SELECT * FROM drop_claims WHERE drop_id = ? AND user_id = ?"
    )
    .bind(&drop_id)
    .bind(&req.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if existing_claim.is_some() {
        return Err(error_response(StatusCode::BAD_REQUEST, "Already claimed".to_string()));
    }

    // Claim作成
    let claim_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO drop_claims (claim_id, drop_id, user_id, device_id_hash, claimed_at) VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&claim_id)
    .bind(&drop_id)
    .bind(&req.user_id)
    .bind(&req.device_id_hash)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    // claimed_count更新
    sqlx::query("UPDATE drops SET claimed_count = claimed_count + 1, updated_at = ? WHERE drop_id = ?")
        .bind(now)
        .bind(&drop_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
        })?;

    info!("Drop claimed: drop_id={}, user_id={}, claim_id={}", drop_id, req.user_id, claim_id);

    // ダウンロードURL生成（簡易トークン）
    let download_url = format!(
        "{}/api/drops/{}/download?token={}",
        state.vps_base_url.replace("/nft", ""),
        drop_id,
        claim_id
    );

    Ok(Json(ClaimDropResponse {
        success: true,
        claim_id,
        drop_id,
        download_url,
        expires_at: drop.end_at,
        audio_sha256: drop.audio_sha256,
        audio_size_bytes: drop.audio_size_bytes,
    }))
}

/// GET /api/drops/:drop_id/download - Dropダウンロード
pub async fn download_drop(
    State(state): State<Arc<AppState>>,
    Path(drop_id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> Result<axum::response::Response<Body>, (StatusCode, Json<ErrorResponse>)> {
    let token = query.token.ok_or_else(|| {
        error_response(StatusCode::UNAUTHORIZED, "Token required".to_string())
    })?;

    // Claim検証
    let claim: Option<DropClaim> = sqlx::query_as(
        "SELECT * FROM drop_claims WHERE claim_id = ? AND drop_id = ?"
    )
    .bind(&token)
    .bind(&drop_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    if claim.is_none() {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Invalid token".to_string()));
    }

    // Drop取得
    let drop: Drop = sqlx::query_as("SELECT * FROM drops WHERE drop_id = ?")
        .bind(&drop_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
        })?;

    // 期限チェック
    let now = chrono::Utc::now().timestamp();
    if now >= drop.end_at {
        return Err(error_response(StatusCode::BAD_REQUEST, "Drop has expired".to_string()));
    }

    // ファイル読み込み
    let audio_path = PathBuf::from(&state.base_data_dir)
        .join("drops")
        .join(&drop.audio_object_key);

    let audio_data = fs::read(&audio_path).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("File read error: {}", e))
    })?;

    // レスポンス構築
    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", &drop.audio_mime)
        .header("Content-Length", audio_data.len())
        .header("Content-Disposition", format!("attachment; filename=\"{}\"", drop.title))
        .body(Body::from(audio_data))
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e))
        })?;

    Ok(response)
}

/// POST /api/vendors/:vendor_stable_id/drops/batch_end - 一括終了
pub async fn batch_end_drops(
    State(state): State<Arc<AppState>>,
    Path(vendor_stable_id): Path<String>,
    Json(req): Json<BatchDropRequest>,
) -> Result<Json<BatchDropResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = chrono::Utc::now().timestamp();
    let mut results = HashMap::new();

    for drop_id in &req.drop_ids {
        let result = sqlx::query(
            "UPDATE drops SET status = ?, ended_at = ?, updated_at = ? WHERE drop_id = ? AND vendor_stable_id = ? AND status IN (?, ?)"
        )
        .bind(drop_status::ENDED)
        .bind(now)
        .bind(now)
        .bind(drop_id)
        .bind(&vendor_stable_id)
        .bind(drop_status::SCHEDULED)
        .bind(drop_status::ACTIVE)
        .execute(&state.db)
        .await;

        results.insert(drop_id.clone(), result.map(|r| r.rows_affected() > 0).unwrap_or(false));
    }

    info!("Batch end drops: vendor={}, count={}", vendor_stable_id, req.drop_ids.len());

    Ok(Json(BatchDropResponse {
        success: true,
        results,
    }))
}

/// POST /api/vendors/:vendor_stable_id/drops/batch_purge - 一括削除
pub async fn batch_purge_drops(
    State(state): State<Arc<AppState>>,
    Path(vendor_stable_id): Path<String>,
    Json(req): Json<BatchDropRequest>,
) -> Result<Json<BatchDropResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = chrono::Utc::now().timestamp();
    let mut results = HashMap::new();

    for drop_id in &req.drop_ids {
        // まずENDEDに（まだの場合）
        let _ = sqlx::query(
            "UPDATE drops SET status = ?, ended_at = COALESCE(ended_at, ?), updated_at = ? WHERE drop_id = ? AND vendor_stable_id = ? AND status IN (?, ?)"
        )
        .bind(drop_status::ENDED)
        .bind(now)
        .bind(now)
        .bind(drop_id)
        .bind(&vendor_stable_id)
        .bind(drop_status::SCHEDULED)
        .bind(drop_status::ACTIVE)
        .execute(&state.db)
        .await;

        // Drop取得
        let drop: Option<Drop> = sqlx::query_as("SELECT * FROM drops WHERE drop_id = ? AND vendor_stable_id = ?")
            .bind(drop_id)
            .bind(&vendor_stable_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

        if let Some(_d) = drop {
            // ファイル削除
            let dir = PathBuf::from(&state.base_data_dir).join("drops").join(drop_id);
            let _ = fs::remove_dir_all(&dir).await;

            // PURGED更新
            let result = sqlx::query(
                "UPDATE drops SET status = ?, purged_at = ?, updated_at = ? WHERE drop_id = ?"
            )
            .bind(drop_status::PURGED)
            .bind(now)
            .bind(now)
            .bind(drop_id)
            .execute(&state.db)
            .await;

            results.insert(drop_id.clone(), result.map(|r| r.rows_affected() > 0).unwrap_or(false));
            info!("Drop purged: drop_id={}", drop_id);
        } else {
            results.insert(drop_id.clone(), false);
        }
    }

    Ok(Json(BatchDropResponse {
        success: true,
        results,
    }))
}

// ========================================
// Background Job (期限切れ自動処理)
// ========================================

/// 期限切れDropsを終了させる（定期実行用）
pub async fn expire_drops(state: &Arc<AppState>) -> anyhow::Result<usize> {
    let now = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        "UPDATE drops SET status = ?, ended_at = ?, updated_at = ? WHERE end_at <= ? AND status IN (?, ?)"
    )
    .bind(drop_status::ENDED)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(drop_status::SCHEDULED)
    .bind(drop_status::ACTIVE)
    .execute(&state.db)
    .await?;

    let count = result.rows_affected() as usize;
    if count > 0 {
        info!("Expired {} drops", count);
    }
    Ok(count)
}

/// 終了済みDropsを削除（定期実行用）
pub async fn purge_ended_drops(state: &Arc<AppState>, grace_seconds: i64) -> anyhow::Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - grace_seconds;

    // 削除対象取得
    let drops: Vec<Drop> = sqlx::query_as(
        "SELECT * FROM drops WHERE status = ? AND ended_at IS NOT NULL AND ended_at <= ?"
    )
    .bind(drop_status::ENDED)
    .bind(cutoff)
    .fetch_all(&state.db)
    .await?;

    let mut count = 0;
    for drop in drops {
        // ファイル削除
        let dir = PathBuf::from(&state.base_data_dir).join("drops").join(&drop.drop_id);
        let _ = fs::remove_dir_all(&dir).await;

        // PURGED更新
        sqlx::query(
            "UPDATE drops SET status = ?, purged_at = ?, updated_at = ? WHERE drop_id = ?"
        )
        .bind(drop_status::PURGED)
        .bind(now)
        .bind(now)
        .bind(&drop.drop_id)
        .execute(&state.db)
        .await?;

        info!("Purged drop: drop_id={}", drop.drop_id);
        count += 1;
    }

    Ok(count)
}

// ========================================
// Helper Functions
// ========================================

fn generate_drop_id() -> String {
    let random_bytes: [u8; 5] = rand::thread_rng().gen();
    let encoded = base32::encode(base32::Alphabet::Crockford, &random_bytes);
    format!("DROP_{}", &encoded[..8])
}

fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    warn!("API Error: {}", message);
    (status, Json(ErrorResponse { success: false, error: message }))
}
