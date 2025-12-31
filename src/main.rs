use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

mod db;
mod models;
mod handlers;

use db::DbPool;

// ========================================
// アプリケーション状態
// ========================================

/// 共有アプリケーション状態
pub struct AppState {
    pub base_data_dir: String,
    pub vps_base_url: String,
    pub db: DbPool,
}

// ========================================
// レガシー設定（後方互換用）
// ========================================

#[derive(Clone)]
struct AppConfig {
    base_data_dir: PathBuf,
    vps_base_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_data_dir: PathBuf::from("/data/nft"),
            vps_base_url: "http://153.121.61.17/nft".to_string(),
        }
    }
}

// ========================================
// レスポンス型
// ========================================

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    service: String,
    version: String,
    db_status: String,
}

#[derive(Serialize)]
struct UploadResponse {
    success: bool,
    url: String,
    path: String,
    filename: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    success: bool,
    error: String,
}

#[derive(Deserialize)]
struct DeleteRequest {
    album_id: String,
    file_type: String, // "promo" | "albums"
}

#[derive(Serialize)]
struct DeleteResponse {
    success: bool,
    message: String,
}

// ========================================
// ハンドラ
// ========================================

/// ヘルスチェック
async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Json<HealthResponse> {
    // DB接続チェック
    let db_status = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "connected".to_string(),
        Err(e) => format!("error: {}", e),
    };

    Json(HealthResponse {
        status: "ok".to_string(),
        service: "nft-upload-api".to_string(),
        version: "0.2.0".to_string(),
        db_status,
    })
}

/// ファイルアップロード（レガシーAPI - 後方互換）
async fn upload_file(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("Multipart parsing started");

    let mut file_data: Option<Vec<u8>> = None;
    let mut original_filename: Option<String> = None;
    let mut album_id: Option<String> = None;
    let mut file_type: Option<String> = None;
    let mut category: Option<String> = None;
    let mut track_number: Option<String> = None;

    // multipart フィールドを解析
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| {
            warn!("Field read error: {:?}", e);
            error_response(StatusCode::BAD_REQUEST, format!("Field read error: {:?}", e))
        })?
    {
        let name = field.name().unwrap_or("").to_string();
        info!("Processing field: {}", name);

        match name.as_str() {
            "file" => {
                original_filename = field.file_name().map(|s| s.to_string());
                info!("File field found: {:?}", original_filename);

                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| {
                        warn!("File bytes read error: {:?}", e);
                        error_response(StatusCode::BAD_REQUEST, format!("File read error: {:?}", e))
                    })?
                    .to_vec();

                info!("File bytes read: {} bytes", bytes.len());
                file_data = Some(bytes);
            }
            "album_id" => {
                let text = field.text().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("album_id error: {:?}", e))
                })?;
                album_id = Some(text);
            }
            "file_type" => {
                let text = field.text().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("file_type error: {:?}", e))
                })?;
                file_type = Some(text);
            }
            "category" => {
                let text = field.text().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("category error: {:?}", e))
                })?;
                category = Some(text);
            }
            "track_number" => {
                let text = field.text().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, format!("track_number error: {:?}", e))
                })?;
                track_number = Some(text);
            }
            _ => {
                warn!("Unknown field: {}", name);
            }
        }
    }

    // 必須パラメータの検証
    let file_data = file_data.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "No file uploaded".to_string())
    })?;

    let original_filename = original_filename.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "No filename provided".to_string())
    })?;

    let album_id = album_id.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "album_id is required".to_string())
    })?;

    let file_type = file_type.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "file_type is required".to_string())
    })?;

    let category = category.ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "category is required".to_string())
    })?;

    // file_type のバリデーション
    if file_type != "promo" && file_type != "albums" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "file_type must be 'promo' or 'albums'".to_string(),
        ));
    }

    // category のバリデーション
    if category != "tracks" && category != "cover" && category != "manifest" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "category must be 'tracks', 'cover', or 'manifest'".to_string(),
        ));
    }

    // ファイル名の生成
    let extension = original_filename
        .split('.')
        .last()
        .unwrap_or("bin")
        .to_lowercase();

    let filename = if category == "tracks" {
        let track_num = track_number.ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "track_number is required for tracks".to_string(),
            )
        })?;
        format!("{}.{}", track_num, extension)
    } else if category == "manifest" {
        "manifest.json".to_string()
    } else {
        format!("cover.{}", extension)
    };

    // 保存先ディレクトリの構築
    let base_dir = PathBuf::from(&state.base_data_dir);
    let target_dir = if category == "tracks" {
        base_dir.join(&file_type).join(&album_id).join("tracks")
    } else {
        base_dir.join(&file_type).join(&album_id)
    };

    // ディレクトリ作成
    fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;

    // ファイル保存
    let target_path = target_dir.join(&filename);
    let mut file = fs::File::create(&target_path).await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create file: {}", e),
        )
    })?;

    file.write_all(&file_data).await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write file: {}", e),
        )
    })?;

    info!("File saved: {:?}", target_path);

    // 所有権を caddy に変更（ベストエフォート）
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        match Command::new("chown")
            .arg("caddy:caddy")
            .arg(&target_path)
            .output()
        {
            Ok(_) => info!("Changed ownership to caddy:caddy"),
            Err(e) => warn!("Failed to chown (not critical): {}", e),
        }
    }

    // URL 生成
    let url = if category == "tracks" {
        format!(
            "{}/{}/{}/tracks/{}",
            state.vps_base_url, file_type, album_id, filename
        )
    } else {
        format!(
            "{}/{}/{}/{}",
            state.vps_base_url, file_type, album_id, filename
        )
    };

    Ok(Json(UploadResponse {
        success: true,
        url,
        path: target_path.to_string_lossy().to_string(),
        filename,
    }))
}

/// ファイル削除（売り切れ時などに使用）
async fn delete_file(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeleteRequest>,
) -> Result<Json<DeleteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let target_dir = PathBuf::from(&state.base_data_dir)
        .join(&payload.file_type)
        .join(&payload.album_id);

    if !target_dir.exists() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            format!("Directory does not exist: {:?}", target_dir),
        ));
    }

    fs::remove_dir_all(&target_dir).await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete directory: {}", e),
        )
    })?;

    info!("Deleted: {:?}", target_dir);

    Ok(Json(DeleteResponse {
        success: true,
        message: format!("Deleted {:?}", target_dir),
    }))
}

// ========================================
// エラーレスポンスヘルパー
// ========================================

fn error_response(status: StatusCode, message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            success: false,
            error: message,
        }),
    )
}

// ========================================
// メイン
// ========================================

#[tokio::main]
async fn main() {
    // ログ初期化
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 設定
    let base_data_dir = "/data/nft".to_string();
    let vps_base_url = "http://153.121.61.17/nft".to_string();
    let db_path = "/data/nft/nft_server.db";

    // DB初期化
    info!("Initializing database...");
    let db = db::init_db(db_path).await.expect("Failed to initialize database");

    // アプリケーション状態
    let state = Arc::new(AppState {
        base_data_dir,
        vps_base_url,
        db,
    });

    // ルーター構築
    let app = Router::new()
        // ヘルスチェック
        .route("/api/health", get(health_check))
        // レガシーAPI（後方互換）
        .route("/api/upload", post(upload_file))
        .route("/api/delete", post(delete_file))
        // Vendors API
        .route("/api/vendors", get(handlers::vendors::list_vendors))
        .route("/api/vendors", post(handlers::vendors::create_vendor))
        .route("/api/vendors/:stable_id", get(handlers::vendors::get_vendor))
        .route("/api/vendors/:stable_id", put(handlers::vendors::update_vendor))
        .route("/api/vendors/:stable_id", delete(handlers::vendors::delist_vendor))
        .route("/api/vendors/:stable_id/icon", post(handlers::vendors::upload_vendor_icon))
        .route("/api/vendors/by-peer/:peer_id", get(handlers::vendors::get_vendor_by_peer))
        // Listings API
        .route("/api/listings", get(handlers::listings::list_listings))
        .route("/api/listings", post(handlers::listings::create_listing))
        .route("/api/listings/:listing_id", get(handlers::listings::get_listing))
        .route("/api/listings/:listing_id", put(handlers::listings::update_listing))
        .route("/api/listings/:listing_id", delete(handlers::listings::delete_listing))
        // Artists API (Account)
        .route("/api/account/artists", get(handlers::artists::list_artists))
        .route("/api/account/artists", post(handlers::artists::create_artist))
        .route("/api/account/artists/:stable_id", get(handlers::artists::get_artist))
        .route("/api/account/artists/:stable_id", put(handlers::artists::update_artist))
        .route("/api/account/artists/:stable_id/icon", post(handlers::artists::upload_artist_icon))
        .route("/api/account/artists/:stable_id/discography", get(handlers::artists::get_discography))
        .route("/api/account/artists/:stable_id/discography", post(handlers::artists::add_discography))
        .route("/api/account/artists/by-peer/:peer_id", get(handlers::artists::get_artist_by_peer))
        // Drops API
        .route("/api/vendors/:vendor_stable_id/drops", get(handlers::drops::list_drops))
        .route("/api/vendors/:vendor_stable_id/drops/batch_end", post(handlers::drops::batch_end_drops))
        .route("/api/vendors/:vendor_stable_id/drops/batch_purge", post(handlers::drops::batch_purge_drops))
        .route("/api/drops", post(handlers::drops::create_drop))
        .route("/api/drops/:drop_id", get(handlers::drops::get_drop))
        .route("/api/drops/:drop_id/claim", post(handlers::drops::claim_drop))
        .route("/api/drops/:drop_id/download", get(handlers::drops::download_drop))
        // ミドルウェア
        .layer(DefaultBodyLimit::max(800 * 1024 * 1024)) // 800MB まで許可
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let addr = "0.0.0.0:3000";
    info!("NFT Upload API Server v0.2.0 listening on {}", addr);
    info!("Max body size: 800MB");
    info!("Database: {}", db_path);

    // 期限切れDrops処理のバックグラウンドジョブ（1時間ごと）
    let state_clone = state;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            info!("[Job] Running expired drops check...");

            // 期限切れDropsをENDED状態に更新
            if let Err(e) = handlers::drops::expire_drops(&state_clone).await {
                warn!("[Job] expire_drops error: {:?}", e);
            }

            // 7日以上前にENDEDになったDropsをpurge（ファイル削除）
            // grace_seconds = 7 * 24 * 3600 = 604800 (7日)
            if let Err(e) = handlers::drops::purge_ended_drops(&state_clone, 604800).await {
                warn!("[Job] purge_ended_drops error: {:?}", e);
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
