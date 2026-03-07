//! Transfer Handler - P2P NFTアルバム転送
//!
//! フロー:
//!   1. POST /api/transfers (multipart) - 送信者が暗号化ファイル + メタデータをアップロード
//!   2. GET  /api/transfers/:transfer_id - 転送情報の取得
//!   3. GET  /api/transfers/:transfer_id/download - 受信者がファイルをDL
//!   4. POST /api/transfers/:transfer_id/claim - 受信者がDL完了を通知 → VPSファイル削除
//!   5. POST /api/transfers/:transfer_id/cancel - 送信者がキャンセル → VPSファイル削除
//!   6. GET  /api/transfers/pending/:peer_id - peer_id宛の未処理転送一覧

use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use uuid::Uuid;

use crate::models::{
    CreateTransferRequest, Transfer, TransferResponse,
    UpdateTransferStatusRequest, transfer_status,
};
use crate::AppState;

/// 期限: 3日（ミリ秒）
const TRANSFER_EXPIRY_MS: i64 = 3 * 24 * 60 * 60 * 1000;

#[derive(Serialize)]
pub struct ErrorResp {
    success: bool,
    error: String,
}

fn err(status: StatusCode, msg: String) -> (StatusCode, Json<ErrorResp>) {
    (status, Json(ErrorResp { success: false, error: msg }))
}

/// POST /api/transfers - 暗号化アルバムデータ + メタデータをアップロード
pub async fn create_transfer(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResp>)> {
    let mut file_data: Option<Vec<u8>> = None;
    let mut metadata_json: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        err(StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                let bytes = field.bytes().await.map_err(|e| {
                    err(StatusCode::BAD_REQUEST, format!("File read error: {}", e))
                })?;
                file_data = Some(bytes.to_vec());
            }
            "metadata" => {
                let text = field.text().await.map_err(|e| {
                    err(StatusCode::BAD_REQUEST, format!("Metadata read error: {}", e))
                })?;
                metadata_json = Some(text);
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| {
        err(StatusCode::BAD_REQUEST, "No file uploaded".into())
    })?;
    let metadata_json = metadata_json.ok_or_else(|| {
        err(StatusCode::BAD_REQUEST, "No metadata provided".into())
    })?;

    let req: CreateTransferRequest = serde_json::from_str(&metadata_json).map_err(|e| {
        err(StatusCode::BAD_REQUEST, format!("Invalid metadata JSON: {}", e))
    })?;

    // Transfer ID 生成
    let transfer_id = format!("TFR_{}", Uuid::new_v4().simple());

    // SHA256
    let sha256 = {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&file_data);
        hex::encode(hasher.finalize())
    };

    // ファイル保存: /data/transfers/{transfer_id}/album.enc
    let transfer_dir = PathBuf::from(&state.base_data_dir)
        .join("transfers")
        .join(&transfer_id);
    fs::create_dir_all(&transfer_dir).await.map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("Dir create error: {}", e))
    })?;

    let data_filename = "album.enc";
    let data_path = transfer_dir.join(data_filename);
    let mut file = fs::File::create(&data_path).await.map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("File create error: {}", e))
    })?;
    file.write_all(&file_data).await.map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("File write error: {}", e))
    })?;

    let data_object_key = format!("{}/{}", transfer_id, data_filename);
    let data_size = file_data.len() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires_at_ms = now_ms + TRANSFER_EXPIRY_MS;

    // DB挿入
    sqlx::query(r#"
        INSERT INTO transfers (
            transfer_id, sender_peer_id, recipient_peer_id,
            nft_object_id, escrow_id, edition_id,
            album_title, album_artist, cover_url, track_count,
            data_object_key, data_size_bytes, data_sha256,
            status, created_at_ms, updated_at_ms, expires_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    "#)
    .bind(&transfer_id)
    .bind(&req.sender_peer_id)
    .bind(&req.recipient_peer_id)
    .bind(&req.nft_object_id)
    .bind(&req.escrow_id)
    .bind(&req.edition_id)
    .bind(&req.album_title)
    .bind(&req.album_artist)
    .bind(&req.cover_url)
    .bind(req.track_count)
    .bind(&data_object_key)
    .bind(data_size)
    .bind(&sha256)
    .bind(transfer_status::PENDING)
    .bind(now_ms)
    .bind(now_ms)
    .bind(expires_at_ms)
    .execute(&state.db)
    .await
    .map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e))
    })?;

    // caddy ownership (Linux only)
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        let _ = Command::new("chown")
            .arg("-R").arg("caddy:caddy")
            .arg(&transfer_dir)
            .output();
    }

    info!("Transfer created: {} ({} bytes)", transfer_id, data_size);

    Ok(Json(TransferResponse {
        transfer_id,
        sender_peer_id: req.sender_peer_id,
        recipient_peer_id: req.recipient_peer_id,
        nft_object_id: req.nft_object_id,
        escrow_id: req.escrow_id,
        edition_id: req.edition_id,
        album_title: req.album_title,
        album_artist: req.album_artist,
        cover_url: req.cover_url,
        track_count: req.track_count,
        download_url: None,
        data_size_bytes: data_size,
        data_sha256: sha256,
        status: transfer_status::PENDING,
        created_at_ms: now_ms,
        expires_at_ms,
    }))
}

/// GET /api/transfers/:transfer_id - 転送情報の取得
pub async fn get_transfer(
    State(state): State<Arc<AppState>>,
    Path(transfer_id): Path<String>,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResp>)> {
    let transfer: Transfer = sqlx::query_as(
        "SELECT * FROM transfers WHERE transfer_id = ?"
    )
    .bind(&transfer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Transfer not found".into()))?;

    let download_url = if transfer.status == transfer_status::PENDING {
        Some(format!("{}/transfers/{}", state.vps_base_url, transfer.data_object_key))
    } else {
        None
    };

    Ok(Json(TransferResponse {
        transfer_id: transfer.transfer_id,
        sender_peer_id: transfer.sender_peer_id,
        recipient_peer_id: transfer.recipient_peer_id,
        nft_object_id: transfer.nft_object_id,
        escrow_id: transfer.escrow_id,
        edition_id: transfer.edition_id,
        album_title: transfer.album_title,
        album_artist: transfer.album_artist,
        cover_url: transfer.cover_url,
        track_count: transfer.track_count,
        download_url,
        data_size_bytes: transfer.data_size_bytes,
        data_sha256: transfer.data_sha256,
        status: transfer.status,
        created_at_ms: transfer.created_at_ms,
        expires_at_ms: transfer.expires_at_ms,
    }))
}

/// GET /api/transfers/:transfer_id/download - ファイルダウンロード（受信者用）
pub async fn download_transfer(
    State(state): State<Arc<AppState>>,
    Path(transfer_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResp>)> {
    let transfer: Transfer = sqlx::query_as(
        "SELECT * FROM transfers WHERE transfer_id = ?"
    )
    .bind(&transfer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Transfer not found".into()))?;

    if transfer.status != transfer_status::PENDING {
        return Err(err(StatusCode::GONE, "Transfer is no longer available".into()));
    }

    let file_path = PathBuf::from(&state.base_data_dir)
        .join("transfers")
        .join(&transfer.data_object_key);

    if !file_path.exists() {
        return Err(err(StatusCode::NOT_FOUND, "Transfer file not found on disk".into()));
    }

    let body = fs::read(&file_path).await.map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("File read error: {}", e))
    })?;

    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Disposition", format!("attachment; filename=\"{}.enc\"", transfer_id))
        .header("X-Data-Sha256", &transfer.data_sha256)
        .body(axum::body::Body::from(body))
        .unwrap())
}

/// POST /api/transfers/:transfer_id/claim - 受信者がDL完了を通知
pub async fn claim_transfer(
    State(state): State<Arc<AppState>>,
    Path(transfer_id): Path<String>,
    Json(req): Json<UpdateTransferStatusRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResp>)> {
    let transfer: Transfer = sqlx::query_as(
        "SELECT * FROM transfers WHERE transfer_id = ?"
    )
    .bind(&transfer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Transfer not found".into()))?;

    // 権限チェック: 受信者のみ
    if req.peer_id != transfer.recipient_peer_id {
        return Err(err(StatusCode::FORBIDDEN, "Only recipient can claim".into()));
    }
    if transfer.status != transfer_status::PENDING {
        return Err(err(StatusCode::CONFLICT, format!("Transfer status is {}, not pending", transfer.status)));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();

    // ステータス更新
    sqlx::query("UPDATE transfers SET status = ?, updated_at_ms = ? WHERE transfer_id = ?")
        .bind(transfer_status::CLAIMED)
        .bind(now_ms)
        .bind(&transfer_id)
        .execute(&state.db)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    // VPSファイル削除
    delete_transfer_files(&state.base_data_dir, &transfer_id).await;

    info!("Transfer claimed: {}", transfer_id);

    Ok(Json(serde_json::json!({
        "success": true,
        "transfer_id": transfer_id,
        "status": transfer_status::CLAIMED
    })))
}

/// POST /api/transfers/:transfer_id/cancel - 送信者がキャンセル
pub async fn cancel_transfer(
    State(state): State<Arc<AppState>>,
    Path(transfer_id): Path<String>,
    Json(req): Json<UpdateTransferStatusRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResp>)> {
    let transfer: Transfer = sqlx::query_as(
        "SELECT * FROM transfers WHERE transfer_id = ?"
    )
    .bind(&transfer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Transfer not found".into()))?;

    // 権限チェック: 送信者のみ
    if req.peer_id != transfer.sender_peer_id {
        return Err(err(StatusCode::FORBIDDEN, "Only sender can cancel".into()));
    }
    if transfer.status != transfer_status::PENDING {
        return Err(err(StatusCode::CONFLICT, format!("Transfer status is {}, not pending", transfer.status)));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();

    sqlx::query("UPDATE transfers SET status = ?, updated_at_ms = ? WHERE transfer_id = ?")
        .bind(transfer_status::CANCELLED)
        .bind(now_ms)
        .bind(&transfer_id)
        .execute(&state.db)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    // VPSファイル削除
    delete_transfer_files(&state.base_data_dir, &transfer_id).await;

    info!("Transfer cancelled: {}", transfer_id);

    Ok(Json(serde_json::json!({
        "success": true,
        "transfer_id": transfer_id,
        "status": transfer_status::CANCELLED
    })))
}

/// GET /api/transfers/pending/:peer_id - 自分宛の未処理転送一覧
pub async fn list_pending_transfers(
    State(state): State<Arc<AppState>>,
    Path(peer_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResp>)> {
    let transfers: Vec<Transfer> = sqlx::query_as(
        "SELECT * FROM transfers WHERE recipient_peer_id = ? AND status = ? ORDER BY created_at_ms DESC"
    )
    .bind(&peer_id)
    .bind(transfer_status::PENDING)
    .fetch_all(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    let responses: Vec<TransferResponse> = transfers.into_iter().map(|t| {
        let download_url = Some(format!("{}/transfers/{}", state.vps_base_url, t.data_object_key));
        TransferResponse {
            transfer_id: t.transfer_id,
            sender_peer_id: t.sender_peer_id,
            recipient_peer_id: t.recipient_peer_id,
            nft_object_id: t.nft_object_id,
            escrow_id: t.escrow_id,
            edition_id: t.edition_id,
            album_title: t.album_title,
            album_artist: t.album_artist,
            cover_url: t.cover_url,
            track_count: t.track_count,
            download_url,
            data_size_bytes: t.data_size_bytes,
            data_sha256: t.data_sha256,
            status: t.status,
            created_at_ms: t.created_at_ms,
            expires_at_ms: t.expires_at_ms,
        }
    }).collect();

    Ok(Json(serde_json::json!({
        "success": true,
        "transfers": responses
    })))
}

// ========================================
// 期限切れ処理（バックグラウンドジョブ用）
// ========================================

/// 期限切れ転送を EXPIRED に更新し、ファイルを削除
pub async fn expire_transfers(state: &Arc<AppState>) -> Result<u64, anyhow::Error> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 期限切れの PENDING を取得
    let expired: Vec<Transfer> = sqlx::query_as(
        "SELECT * FROM transfers WHERE status = ? AND expires_at_ms <= ?"
    )
    .bind(transfer_status::PENDING)
    .bind(now_ms)
    .fetch_all(&state.db)
    .await?;

    let count = expired.len() as u64;

    for t in &expired {
        // ステータス更新
        sqlx::query("UPDATE transfers SET status = ?, updated_at_ms = ? WHERE transfer_id = ?")
            .bind(transfer_status::EXPIRED)
            .bind(now_ms)
            .bind(&t.transfer_id)
            .execute(&state.db)
            .await?;

        // ファイル削除
        delete_transfer_files(&state.base_data_dir, &t.transfer_id).await;

        info!("Transfer expired: {}", t.transfer_id);
    }

    Ok(count)
}

/// 古い転送レコードをパージ（7日以上前に完了/キャンセル/期限切れ）
pub async fn purge_old_transfers(state: &Arc<AppState>, grace_ms: i64) -> Result<u64, anyhow::Error> {
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - grace_ms;

    let result = sqlx::query(
        "DELETE FROM transfers WHERE status IN (?, ?, ?) AND updated_at_ms < ?"
    )
    .bind(transfer_status::CLAIMED)
    .bind(transfer_status::CANCELLED)
    .bind(transfer_status::EXPIRED)
    .bind(cutoff_ms)
    .execute(&state.db)
    .await?;

    Ok(result.rows_affected())
}

// ========================================
// ヘルパー
// ========================================

/// 転送ファイルを削除（ベストエフォート）
async fn delete_transfer_files(base_data_dir: &str, transfer_id: &str) {
    let transfer_dir = PathBuf::from(base_data_dir)
        .join("transfers")
        .join(transfer_id);

    if transfer_dir.exists() {
        match fs::remove_dir_all(&transfer_dir).await {
            Ok(_) => info!("Deleted transfer files: {}", transfer_id),
            Err(e) => warn!("Failed to delete transfer files {}: {}", transfer_id, e),
        }
    }
}
