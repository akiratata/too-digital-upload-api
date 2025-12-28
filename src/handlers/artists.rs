//! Artists API Handlers
//! /api/account/artists エンドポイント

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

use crate::models::{
    CreateArtistRequest, UpdateArtistRequest, Artist, ArtistProfile, ArtistP2P,
    ArtistResponse, ArtistCreateResponse, AddDiscographyRequest, DiscographyEntry,
    DiscographyJson, DiscographyAlbum, TrackPreview,
};
use crate::AppState;

// ========================================
// Response Types
// ========================================

#[derive(Serialize)]
pub struct ArtistListResponse {
    pub success: bool,
    pub artists: Vec<ArtistResponse>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct ArtistDetailResponse {
    pub success: bool,
    pub artist: Option<ArtistResponse>,
}

#[derive(Serialize)]
pub struct DiscographyResponse {
    pub success: bool,
    pub discography: DiscographyJson,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
}

// ========================================
// Handlers
// ========================================

/// GET /api/account/artists - Artist一覧取得
pub async fn list_artists(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ArtistListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let artists: Vec<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE is_alive = 1 ORDER BY created_at_ms DESC"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    let mut responses = Vec::new();
    for a in &artists {
        let profile = load_artist_profile(&state.base_data_dir, &a.stable_id).await.ok();
        responses.push(artist_to_response(a, profile));
    }

    let total = responses.len();
    Ok(Json(ArtistListResponse {
        success: true,
        artists: responses,
        total,
    }))
}

/// GET /api/account/artists/:stable_id - Artist詳細取得
pub async fn get_artist(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
) -> Result<Json<ArtistDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let artist: Option<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    match artist {
        Some(a) => {
            let profile = load_artist_profile(&state.base_data_dir, &a.stable_id).await.ok();
            Ok(Json(ArtistDetailResponse {
                success: true,
                artist: Some(artist_to_response(&a, profile)),
            }))
        }
        None => Err(error_response(StatusCode::NOT_FOUND, "Artist not found".to_string())),
    }
}

/// GET /api/account/artists/by-peer/:peer_id - peer_idでArtist取得
pub async fn get_artist_by_peer(
    State(state): State<Arc<AppState>>,
    Path(peer_id): Path<String>,
) -> Result<Json<ArtistDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let artist: Option<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE peer_id = ?"
    )
    .bind(&peer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    match artist {
        Some(a) => {
            let profile = load_artist_profile(&state.base_data_dir, &a.stable_id).await.ok();
            Ok(Json(ArtistDetailResponse {
                success: true,
                artist: Some(artist_to_response(&a, profile)),
            }))
        }
        None => Err(error_response(StatusCode::NOT_FOUND, "Artist not found for this peer_id".to_string())),
    }
}

/// POST /api/account/artists - Artist作成
pub async fn create_artist(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateArtistRequest>,
) -> Result<Json<ArtistCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // peer_id の重複チェック
    let existing: Option<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE peer_id = ?"
    )
    .bind(&req.peer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    if let Some(a) = existing {
        // 既存を返す（冪等性）
        info!("Artist already exists for peer_id: {} -> stable_id: {}", req.peer_id, a.stable_id);
        return Ok(Json(ArtistCreateResponse {
            success: true,
            stable_id: a.stable_id.clone(),
            peer_id: a.peer_id.clone(),
            profile_url: a.profile_url.unwrap_or_default(),
            profile_sha256: a.profile_sha256.unwrap_or_default(),
            discography_url: a.discography_url.unwrap_or_default(),
            discography_sha256: a.discography_sha256.unwrap_or_default(),
            icon_url: None,
            updated_at_ms: a.updated_at_ms.unwrap_or(now_ms),
        }));
    }

    // stable_id 生成 (ARTIST_ + base32 short)
    let stable_id = generate_stable_id();

    // peer_id_sha256 計算
    let peer_id_sha256 = compute_sha256(&req.peer_id);

    // ディレクトリ作成
    let artist_dir = PathBuf::from(&state.base_data_dir)
        .parent().unwrap_or(&PathBuf::from("/data"))
        .join("account")
        .join("artists")
        .join(&stable_id);
    fs::create_dir_all(&artist_dir).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {}", e))
    })?;

    // profile.json 保存
    let profile = ArtistProfile {
        version: "1.0".to_string(),
        stable_id: stable_id.clone(),
        name: req.name.clone(),
        bio: req.bio.clone(),
        icon_url: None,
        links: vec![],
        p2p: Some(ArtistP2P {
            peer_id: req.peer_id.clone(),
            peer_id_sha256: Some(peer_id_sha256.clone()),
        }),
        updated_at_ms: now_ms,
    };
    let (profile_url, profile_sha256) = save_artist_profile(
        &state.base_data_dir,
        &state.vps_base_url,
        &stable_id,
        &profile,
    ).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save profile: {}", e))
    })?;

    // discography.json 初期生成（空）
    let discography = DiscographyJson {
        version: "1.1".to_string(),
        artist_stable_id: stable_id.clone(),
        albums: vec![],
        updated_at_ms: now_ms,
    };
    let (discography_url, discography_sha256) = save_discography_json(
        &state.base_data_dir,
        &state.vps_base_url,
        &stable_id,
        &discography,
    ).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save discography: {}", e))
    })?;

    // DBに挿入
    sqlx::query(r#"
        INSERT INTO artists (
            stable_id, peer_id, peer_id_sha256, latest_object_id, owner,
            profile_url, profile_sha256, discography_url, discography_sha256,
            profile_seq, status, env, created_at_ms, updated_at_ms, is_alive
        ) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, 1, 0, ?, ?, ?, 1)
    "#)
    .bind(&stable_id)
    .bind(&req.peer_id)
    .bind(&peer_id_sha256)
    .bind(&req.owner)
    .bind(&profile_url)
    .bind(&profile_sha256)
    .bind(&discography_url)
    .bind(&discography_sha256)
    .bind(&req.env)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    info!("Artist created: stable_id={}, peer_id={}", stable_id, req.peer_id);

    Ok(Json(ArtistCreateResponse {
        success: true,
        stable_id,
        peer_id: req.peer_id,
        profile_url,
        profile_sha256,
        discography_url,
        discography_sha256,
        icon_url: None,
        updated_at_ms: now_ms,
    }))
}

/// PUT /api/account/artists/:stable_id - Artist更新
pub async fn update_artist(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
    Json(req): Json<UpdateArtistRequest>,
) -> Result<Json<ArtistCreateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 既存チェック
    let existing: Option<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    let artist = existing.ok_or_else(|| {
        error_response(StatusCode::NOT_FOUND, "Artist not found".to_string())
    })?;

    // profile.json 更新
    let mut profile = load_artist_profile(&state.base_data_dir, &stable_id)
        .await
        .unwrap_or_else(|_| ArtistProfile {
            version: "1.0".to_string(),
            stable_id: stable_id.clone(),
            name: "Unknown".to_string(),
            bio: None,
            icon_url: None,
            links: vec![],
            p2p: Some(ArtistP2P {
                peer_id: artist.peer_id.clone(),
                peer_id_sha256: artist.peer_id_sha256.clone(),
            }),
            updated_at_ms: now_ms,
        });

    if let Some(name) = &req.name {
        profile.name = name.clone();
    }
    if let Some(bio) = &req.bio {
        profile.bio = Some(bio.clone());
    }
    profile.updated_at_ms = now_ms;

    let (profile_url, profile_sha256) = save_artist_profile(
        &state.base_data_dir,
        &state.vps_base_url,
        &stable_id,
        &profile,
    ).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save profile: {}", e))
    })?;

    // DB更新
    sqlx::query(r#"
        UPDATE artists SET
            latest_object_id = COALESCE(?, latest_object_id),
            owner = COALESCE(?, owner),
            profile_url = ?,
            profile_sha256 = ?,
            profile_seq = profile_seq + 1,
            status = COALESCE(?, status),
            updated_at_ms = ?
        WHERE stable_id = ?
    "#)
    .bind(&req.object_id)
    .bind(&req.owner)
    .bind(&profile_url)
    .bind(&profile_sha256)
    .bind(req.status)
    .bind(now_ms)
    .bind(&stable_id)
    .execute(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    info!("Artist updated: stable_id={}", stable_id);

    Ok(Json(ArtistCreateResponse {
        success: true,
        stable_id,
        peer_id: artist.peer_id,
        profile_url,
        profile_sha256,
        discography_url: artist.discography_url.unwrap_or_default(),
        discography_sha256: artist.discography_sha256.unwrap_or_default(),
        icon_url: profile.icon_url,
        updated_at_ms: now_ms,
    }))
}

/// POST /api/account/artists/:stable_id/icon - アイコンアップロード
pub async fn upload_artist_icon(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
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
                .parent().unwrap_or(&PathBuf::from("/data"))
                .join("account")
                .join("artists")
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

            // icon_url を profile.json に更新
            let icon_url = format!(
                "{}/account/artists/{}/{}",
                state.vps_base_url.replace("/nft", ""),
                stable_id,
                icon_filename
            );

            // profile.json を更新
            if let Ok(mut profile) = load_artist_profile(&state.base_data_dir, &stable_id).await {
                profile.icon_url = Some(icon_url.clone());
                profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
                let _ = save_artist_profile(
                    &state.base_data_dir,
                    &state.vps_base_url,
                    &stable_id,
                    &profile,
                ).await;
            }

            info!("Icon uploaded: {}", icon_url);

            return Ok(Json(serde_json::json!({
                "success": true,
                "icon_url": icon_url,
                "path": path.to_string_lossy()
            })));
        }
    }

    Err(error_response(StatusCode::BAD_REQUEST, "No file provided".to_string()))
}

/// POST /api/account/artists/:stable_id/discography - ディスコグラフィ追加
pub async fn add_discography(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
    Json(req): Json<AddDiscographyRequest>,
) -> Result<Json<DiscographyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // Artist 存在チェック
    let artist: Option<Artist> = sqlx::query_as(
        "SELECT * FROM artists WHERE stable_id = ?"
    )
    .bind(&stable_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    if artist.is_none() {
        return Err(error_response(StatusCode::NOT_FOUND, "Artist not found".to_string()));
    }

    // track_preview を JSON 文字列に変換
    let track_preview_json = serde_json::to_string(&req.track_preview).unwrap_or("[]".to_string());

    // DB に UPSERT
    sqlx::query(r#"
        INSERT INTO discography (
            artist_stable_id, album_id, edition_id, title, cover_thumb_url,
            track_count, track_preview, role, deployed_at_ms, created_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(artist_stable_id, album_id) DO UPDATE SET
            edition_id = excluded.edition_id,
            title = excluded.title,
            cover_thumb_url = excluded.cover_thumb_url,
            track_count = excluded.track_count,
            track_preview = excluded.track_preview,
            role = excluded.role,
            deployed_at_ms = excluded.deployed_at_ms
    "#)
    .bind(&stable_id)
    .bind(&req.album_id)
    .bind(&req.edition_id)
    .bind(&req.title)
    .bind(&req.cover_thumb_url)
    .bind(req.track_count)
    .bind(&track_preview_json)
    .bind(&req.role)
    .bind(req.deployed_at_ms.unwrap_or(now_ms))
    .bind(now_ms)
    .execute(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    // discography.json を再生成
    let discography = regenerate_discography(&state, &stable_id, now_ms).await?;

    info!("Discography added: artist={}, album={}", stable_id, req.album_id);

    Ok(Json(DiscographyResponse {
        success: true,
        discography,
    }))
}

/// GET /api/account/artists/:stable_id/discography - ディスコグラフィ取得
pub async fn get_discography(
    State(state): State<Arc<AppState>>,
    Path(stable_id): Path<String>,
) -> Result<Json<DiscographyResponse>, (StatusCode, Json<ErrorResponse>)> {
    // discography.json を読み込み
    let discography = load_discography_json(&state.base_data_dir, &stable_id).await
        .map_err(|_| error_response(StatusCode::NOT_FOUND, "Discography not found".to_string()))?;

    Ok(Json(DiscographyResponse {
        success: true,
        discography,
    }))
}

// ========================================
// Helper Functions
// ========================================

/// stable_id 生成 (ARTIST_ + base32 8文字)
fn generate_stable_id() -> String {
    use rand::Rng;
    let random_bytes: [u8; 5] = rand::thread_rng().gen();
    let encoded = base32::encode(base32::Alphabet::Crockford, &random_bytes);
    format!("ARTIST_{}", &encoded[..8])
}

/// SHA256 計算
fn compute_sha256(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

/// ArtistProfile を保存して URL と SHA256 を返す
async fn save_artist_profile(
    base_dir: &str,
    base_url: &str,
    stable_id: &str,
    profile: &ArtistProfile,
) -> anyhow::Result<(String, String)> {
    let dir = PathBuf::from(base_dir)
        .parent().unwrap_or(&PathBuf::from("/data"))
        .join("account")
        .join("artists")
        .join(stable_id);
    fs::create_dir_all(&dir).await?;

    let json = serde_json::to_string_pretty(profile)?;

    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let sha256 = hex::encode(hasher.finalize());

    let path = dir.join("profile.json");
    let mut file = fs::File::create(&path).await?;
    file.write_all(json.as_bytes()).await?;

    let url = format!(
        "{}/account/artists/{}/profile.json",
        base_url.replace("/nft", ""),
        stable_id
    );

    info!("Artist profile saved: {} (sha256: {})", url, &sha256[..16]);
    Ok((url, sha256))
}

/// ArtistProfile をファイルから読み込む
async fn load_artist_profile(base_dir: &str, stable_id: &str) -> anyhow::Result<ArtistProfile> {
    let path = PathBuf::from(base_dir)
        .parent().unwrap_or(&PathBuf::from("/data"))
        .join("account")
        .join("artists")
        .join(stable_id)
        .join("profile.json");

    let content = fs::read_to_string(&path).await?;
    let profile: ArtistProfile = serde_json::from_str(&content)?;
    Ok(profile)
}

/// DiscographyJson を保存
async fn save_discography_json(
    base_dir: &str,
    base_url: &str,
    stable_id: &str,
    discography: &DiscographyJson,
) -> anyhow::Result<(String, String)> {
    let dir = PathBuf::from(base_dir)
        .parent().unwrap_or(&PathBuf::from("/data"))
        .join("account")
        .join("artists")
        .join(stable_id);
    fs::create_dir_all(&dir).await?;

    let json = serde_json::to_string_pretty(discography)?;

    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let sha256 = hex::encode(hasher.finalize());

    let path = dir.join("discography.json");
    let mut file = fs::File::create(&path).await?;
    file.write_all(json.as_bytes()).await?;

    let url = format!(
        "{}/account/artists/{}/discography.json",
        base_url.replace("/nft", ""),
        stable_id
    );

    info!("Discography saved: {} (sha256: {})", url, &sha256[..16]);
    Ok((url, sha256))
}

/// DiscographyJson をファイルから読み込む
async fn load_discography_json(base_dir: &str, stable_id: &str) -> anyhow::Result<DiscographyJson> {
    let path = PathBuf::from(base_dir)
        .parent().unwrap_or(&PathBuf::from("/data"))
        .join("account")
        .join("artists")
        .join(stable_id)
        .join("discography.json");

    let content = fs::read_to_string(&path).await?;
    let discography: DiscographyJson = serde_json::from_str(&content)?;
    Ok(discography)
}

/// DB から discography を読み直して JSON を再生成
async fn regenerate_discography(
    state: &Arc<AppState>,
    stable_id: &str,
    now_ms: i64,
) -> Result<DiscographyJson, (StatusCode, Json<ErrorResponse>)> {
    let entries: Vec<DiscographyEntry> = sqlx::query_as(
        "SELECT * FROM discography WHERE artist_stable_id = ? ORDER BY deployed_at_ms DESC"
    )
    .bind(stable_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    let albums: Vec<DiscographyAlbum> = entries.iter().map(|e| {
        let track_preview: Vec<TrackPreview> = e.track_preview
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        DiscographyAlbum {
            album_id: e.album_id.clone(),
            edition_id: e.edition_id.clone(),
            title: e.title.clone(),
            cover_thumb_url: e.cover_thumb_url.clone(),
            track_count: e.track_count,
            track_preview,
            deployed_at_ms: e.deployed_at_ms,
            role: e.role.clone(),
        }
    }).collect();

    let discography = DiscographyJson {
        version: "1.1".to_string(),
        artist_stable_id: stable_id.to_string(),
        albums,
        updated_at_ms: now_ms,
    };

    // ファイルに保存
    let (discography_url, discography_sha256) = save_discography_json(
        &state.base_data_dir,
        &state.vps_base_url,
        stable_id,
        &discography,
    ).await.map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save discography: {}", e))
    })?;

    // DB の discography_url/sha256 を更新
    sqlx::query(r#"
        UPDATE artists SET
            discography_url = ?,
            discography_sha256 = ?,
            updated_at_ms = ?
        WHERE stable_id = ?
    "#)
    .bind(&discography_url)
    .bind(&discography_sha256)
    .bind(now_ms)
    .bind(stable_id)
    .execute(&state.db)
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?;

    Ok(discography)
}

/// Artist を ArtistResponse に変換
fn artist_to_response(a: &Artist, profile: Option<ArtistProfile>) -> ArtistResponse {
    ArtistResponse {
        stable_id: a.stable_id.clone(),
        peer_id: a.peer_id.clone(),
        object_id: a.latest_object_id.clone(),
        owner: a.owner.clone(),
        profile,
        profile_url: a.profile_url.clone(),
        profile_sha256: a.profile_sha256.clone(),
        discography_url: a.discography_url.clone(),
        discography_sha256: a.discography_sha256.clone(),
        profile_seq: a.profile_seq,
        status: a.status,
        created_at_ms: a.created_at_ms,
        updated_at_ms: a.updated_at_ms,
        is_alive: a.is_alive == 1,
    }
}

/// エラーレスポンス生成
fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    warn!("API Error: {}", message);
    (status, Json(ErrorResponse { success: false, error: message }))
}
