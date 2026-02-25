//! Devices API Handlers
//! /api/devices エンドポイント
//! デバイス制限: 1 peer_id → PC1台 + Mobile1台

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::models::{
    Device, RegisterDeviceRequest, DeviceResponse, DeviceListResponse, RegisterDeviceResponse,
};
use crate::AppState;

// ========================================
// Response Types
// ========================================

#[derive(Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize)]
pub struct SuccessResponse {
    pub success: bool,
}

fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    warn!("Device API Error: {}", message);
    (
        status,
        Json(ErrorResponse {
            success: false,
            error: message,
        }),
    )
}

fn device_to_response(d: &Device) -> DeviceResponse {
    DeviceResponse {
        device_id: d.device_id.clone(),
        peer_id: d.peer_id.clone(),
        device_type: d.device_type.clone(),
        device_name: d.device_name.clone(),
        platform: d.platform.clone(),
        registered_at_ms: d.registered_at_ms,
        last_seen_at_ms: d.last_seen_at_ms,
    }
}

// ========================================
// Handlers
// ========================================

/// POST /api/devices/register - デバイス登録
///
/// 同一 peer_id + device_type でアクティブなデバイスが既にあり、
/// device_id が異なる場合は 403 で拒否（スロット使用中）。
/// 同一 device_id なら last_seen_at_ms を更新（再登録/heartbeat）。
pub async fn register_device(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterDeviceRequest>,
) -> Result<Json<RegisterDeviceResponse>, (StatusCode, Json<ErrorResponse>)> {
    // バリデーション
    if req.device_type != "pc" && req.device_type != "mobile" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "device_type must be 'pc' or 'mobile'".to_string(),
        ));
    }

    if req.peer_id.is_empty() || req.device_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "peer_id and device_id are required".to_string(),
        ));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();

    // 同一 peer_id + device_type でアクティブなデバイスを検索
    let existing: Option<Device> = sqlx::query_as(
        "SELECT * FROM devices WHERE peer_id = ? AND device_type = ? AND is_alive = 1",
    )
    .bind(&req.peer_id)
    .bind(&req.device_type)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    if let Some(existing_device) = existing {
        if existing_device.device_id == req.device_id {
            // 同じデバイス → last_seen_at_ms を更新
            sqlx::query("UPDATE devices SET last_seen_at_ms = ?, device_name = ? WHERE device_id = ?")
                .bind(now_ms)
                .bind(&req.device_name)
                .bind(&req.device_id)
                .execute(&state.db)
                .await
                .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

            info!("[Device] Heartbeat: {} ({})", req.device_id, req.device_type);
        } else {
            // 別のデバイス → スロット使用中、拒否
            return Err(error_response(
                StatusCode::FORBIDDEN,
                format!(
                    "Device slot '{}' is already in use by '{}'. Unregister it first.",
                    req.device_type, existing_device.device_name
                ),
            ));
        }
    } else {
        // 新規登録
        sqlx::query(
            r#"INSERT INTO devices (device_id, peer_id, device_type, device_name, platform, registered_at_ms, last_seen_at_ms, is_alive)
               VALUES (?, ?, ?, ?, ?, ?, ?, 1)"#,
        )
        .bind(&req.device_id)
        .bind(&req.peer_id)
        .bind(&req.device_type)
        .bind(&req.device_name)
        .bind(&req.platform)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&state.db)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

        info!(
            "[Device] Registered: {} ({}) for peer {}",
            req.device_name, req.device_type, req.peer_id
        );
    }

    // スロット状態を取得
    let (pc_slot, mobile_slot) = get_slot_status(&state, &req.peer_id).await;

    // 登録されたデバイスの情報を返す
    let device: Device = sqlx::query_as("SELECT * FROM devices WHERE device_id = ?")
        .bind(&req.device_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    Ok(Json(RegisterDeviceResponse {
        success: true,
        device: device_to_response(&device),
        pc_slot_used: pc_slot,
        mobile_slot_used: mobile_slot,
    }))
}

/// GET /api/devices/:peer_id - デバイス一覧取得
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
    Path(peer_id): Path<String>,
) -> Result<Json<DeviceListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let devices: Vec<Device> = sqlx::query_as(
        "SELECT * FROM devices WHERE peer_id = ? AND is_alive = 1 ORDER BY registered_at_ms",
    )
    .bind(&peer_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    let pc_slot = devices.iter().any(|d| d.device_type == "pc");
    let mobile_slot = devices.iter().any(|d| d.device_type == "mobile");

    Ok(Json(DeviceListResponse {
        success: true,
        devices: devices.iter().map(device_to_response).collect(),
        pc_slot_used: pc_slot,
        mobile_slot_used: mobile_slot,
    }))
}

/// DELETE /api/devices/:peer_id/:device_type - デバイス登録解除
///
/// 機種変更時に古いデバイスのスロットを解放する。
pub async fn unregister_device(
    State(state): State<Arc<AppState>>,
    Path((peer_id, device_type)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    if device_type != "pc" && device_type != "mobile" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "device_type must be 'pc' or 'mobile'".to_string(),
        ));
    }

    let result = sqlx::query(
        "UPDATE devices SET is_alive = 0 WHERE peer_id = ? AND device_type = ? AND is_alive = 1",
    )
    .bind(&peer_id)
    .bind(&device_type)
    .execute(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    if result.rows_affected() == 0 {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            format!("No active {} device found for this peer", device_type),
        ));
    }

    info!("[Device] Unregistered: {} slot for peer {}", device_type, peer_id);

    Ok(Json(SuccessResponse { success: true }))
}

// ========================================
// Helpers
// ========================================

async fn get_slot_status(state: &Arc<AppState>, peer_id: &str) -> (bool, bool) {
    let devices: Vec<Device> = sqlx::query_as(
        "SELECT * FROM devices WHERE peer_id = ? AND is_alive = 1",
    )
    .bind(peer_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let pc = devices.iter().any(|d| d.device_type == "pc");
    let mobile = devices.iter().any(|d| d.device_type == "mobile");
    (pc, mobile)
}
