//! Devices API Handlers
//! /api/devices エンドポイント
//! デバイス制限: 1 peer_id → PC1台 + Mobile1台
//! Challenge-Response Ed25519認証付き

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use base64::Engine;
use ed25519_dalek::{Verifier, VerifyingKey, Signature};
use rand::RngCore;
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::models::{
    Device, RegisterDeviceRequest, DeviceResponse, DeviceListResponse, RegisterDeviceResponse,
    DeviceChallengeResponse, DeviceVerifyRequest, DeviceVerifyResponse,
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
// Auth: Challenge-Response
// ========================================

/// Ed25519公開鍵からlibp2p PeerIDを導出
///
/// toodigital_rust/src/identity/person_key.rs の pubkey_to_libp2p_peer_id と同一ロジック
fn derive_peer_id_from_pubkey(public_key: &[u8; 32]) -> String {
    // Protobuf encoding for Ed25519 public key
    let mut protobuf = Vec::with_capacity(36);
    protobuf.push(0x08); // field 1, wire type 0
    protobuf.push(0x01); // Ed25519
    protobuf.push(0x12); // field 2, wire type 2
    protobuf.push(0x20); // length 32
    protobuf.extend_from_slice(public_key);

    // Identity multihash
    let mut multihash = Vec::with_capacity(38);
    multihash.push(0x00); // identity hash function
    multihash.push(protobuf.len() as u8); // digest size
    multihash.extend_from_slice(&protobuf);

    // Base58btc encode
    bs58::encode(&multihash).into_string()
}

/// Bearerトークンからpeer_idを抽出・検証
async fn extract_auth_peer_id(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Authorization header required".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Invalid authorization format".to_string()))?;

    let tokens = state.tokens.read().await;
    let (peer_id, expires_at_ms) = tokens.get(token).ok_or_else(|| {
        error_response(StatusCode::UNAUTHORIZED, "Invalid or expired token".to_string())
    })?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    if *expires_at_ms < now_ms {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Token expired".to_string()));
    }

    Ok(peer_id.clone())
}

/// GET /api/devices/auth/challenge — Challenge取得
///
/// ランダム32バイトのhex文字列を返す（5分有効）
pub async fn get_challenge(
    State(state): State<Arc<AppState>>,
) -> Json<DeviceChallengeResponse> {
    let challenge = {
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires_at_ms = now_ms + 5 * 60 * 1000; // 5分

    // Challenge保存
    let mut challenges = state.challenges.write().await;
    challenges.insert(challenge.clone(), (challenge.clone(), expires_at_ms));

    info!("[DeviceAuth] Challenge issued (expires in 5min)");

    Json(DeviceChallengeResponse {
        challenge,
        expires_at_ms,
    })
}

/// POST /api/devices/auth/verify — 署名検証 → トークン発行
///
/// 1. challengeの存在・有効期限確認
/// 2. Ed25519署名検証
/// 3. 公開鍵からpeer_id導出
/// 4. リクエストのpeer_idと一致確認
/// 5. トークン発行（1時間有効）
pub async fn verify_challenge(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeviceVerifyRequest>,
) -> Result<Json<DeviceVerifyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 1. Challenge確認
    {
        let mut challenges = state.challenges.write().await;
        let entry = challenges.get(&req.challenge).ok_or_else(|| {
            error_response(StatusCode::BAD_REQUEST, "Unknown or expired challenge".to_string())
        })?;

        if entry.1 < now_ms {
            challenges.remove(&req.challenge);
            return Err(error_response(StatusCode::BAD_REQUEST, "Challenge expired".to_string()));
        }

        // 使用済みchallengeを削除（再利用防止）
        challenges.remove(&req.challenge);
    }

    // 2. 公開鍵デコード
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(&req.pubkey)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, format!("Invalid pubkey base64: {}", e)))?;

    if pubkey_bytes.len() != 32 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Public key must be 32 bytes, got {}", pubkey_bytes.len()),
        ));
    }

    let pubkey_array: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid pubkey length".to_string()))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, format!("Invalid public key: {}", e)))?;

    // 3. 署名デコード・検証
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&req.sig)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, format!("Invalid sig base64: {}", e)))?;

    if sig_bytes.len() != 64 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Signature must be 64 bytes, got {}", sig_bytes.len()),
        ));
    }

    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid sig length".to_string()))?;

    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(req.challenge.as_bytes(), &signature)
        .map_err(|_| error_response(StatusCode::UNAUTHORIZED, "Signature verification failed".to_string()))?;

    // 4. 公開鍵からpeer_id導出 → リクエストと一致確認
    let derived_peer_id = derive_peer_id_from_pubkey(&pubkey_array);
    if derived_peer_id != req.peer_id {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "peer_id does not match public key".to_string(),
        ));
    }

    // 5. トークン発行（1時間有効）
    let token = {
        let mut rng = rand::thread_rng();
        let mut token_bytes = [0u8; 32];
        rng.fill_bytes(&mut token_bytes);
        hex::encode(token_bytes)
    };
    let token_expires_at_ms = now_ms + 3600 * 1000; // 1時間

    {
        let mut tokens = state.tokens.write().await;
        tokens.insert(token.clone(), (req.peer_id.clone(), token_expires_at_ms));
    }

    info!("[DeviceAuth] Token issued for peer {}", req.peer_id);

    Ok(Json(DeviceVerifyResponse {
        ok: true,
        token,
        peer_id: req.peer_id,
        expires_at_ms: token_expires_at_ms,
    }))
}

/// 期限切れchallenge/tokenクリーンアップ
pub async fn cleanup_expired_auth(state: &Arc<AppState>) {
    let now_ms = chrono::Utc::now().timestamp_millis();

    {
        let mut challenges = state.challenges.write().await;
        let before = challenges.len();
        challenges.retain(|_, (_, expires)| *expires > now_ms);
        let removed = before - challenges.len();
        if removed > 0 {
            info!("[DeviceAuth] Cleaned up {} expired challenge(s)", removed);
        }
    }

    {
        let mut tokens = state.tokens.write().await;
        let before = tokens.len();
        tokens.retain(|_, (_, expires)| *expires > now_ms);
        let removed = before - tokens.len();
        if removed > 0 {
            info!("[DeviceAuth] Cleaned up {} expired token(s)", removed);
        }
    }
}

// ========================================
// Handlers (認証付き)
// ========================================

/// POST /api/devices/register - デバイス登録（認証必須）
///
/// 同一 peer_id + device_type でアクティブなデバイスが既にあり、
/// device_id が異なる場合は 403 で拒否（スロット使用中）。
/// 同一 device_id なら last_seen_at_ms を更新（再登録/heartbeat）。
pub async fn register_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RegisterDeviceRequest>,
) -> Result<Json<RegisterDeviceResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 認証
    let auth_peer_id = extract_auth_peer_id(&state, &headers).await?;

    // リクエストのpeer_idとトークンのpeer_idが一致するか確認
    if auth_peer_id != req.peer_id {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Token peer_id does not match request peer_id".to_string(),
        ));
    }

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

/// GET /api/devices/:peer_id - デバイス一覧取得（認証必須）
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(peer_id): Path<String>,
) -> Result<Json<DeviceListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 認証
    let auth_peer_id = extract_auth_peer_id(&state, &headers).await?;

    if auth_peer_id != peer_id {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Token peer_id does not match requested peer_id".to_string(),
        ));
    }

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

/// DELETE /api/devices/:peer_id/:device_type - デバイス登録解除（認証必須）
///
/// 機種変更時に古いデバイスのスロットを解放する。
pub async fn unregister_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((peer_id, device_type)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 認証
    let auth_peer_id = extract_auth_peer_id(&state, &headers).await?;

    if auth_peer_id != peer_id {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Token peer_id does not match requested peer_id".to_string(),
        ));
    }

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
// Background Jobs
// ========================================

/// 期限切れデバイスを無効化（TTL超過）
///
/// heartbeat（last_seen_at_ms）がttl_ms以上前のデバイスをis_alive=0にする
pub async fn expire_stale_devices(state: &Arc<AppState>, ttl_ms: i64) -> Result<usize, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff_ms = now_ms - ttl_ms;

    let result = sqlx::query(
        "UPDATE devices SET is_alive = 0 WHERE is_alive = 1 AND last_seen_at_ms < ?",
    )
    .bind(cutoff_ms)
    .execute(&state.db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let count = result.rows_affected() as usize;
    if count > 0 {
        info!("[Device] Expired {} stale device(s) (cutoff={}ms ago)", count, ttl_ms);
    }

    Ok(count)
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
