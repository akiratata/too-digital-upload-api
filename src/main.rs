use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

// ========================================
// 設定
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
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        service: "nft-upload-api".to_string(),
        version: "0.1.0".to_string(),
    })
}

/// ファイルアップロード
///
/// Parameters (multipart/form-data):
///   - file: バイナリファイル（必須）
///   - album_id: アルバムID（必須）例: "album123"
///   - file_type: "promo" | "albums"（必須）
///   - category: "tracks" | "cover"（必須）
///   - track_number: トラック番号（tracks の場合のみ）例: "01"
///
/// Returns:
///   JSON: {
///     "success": true,
///     "url": "http://153.121.61.17/nft/promo/album123/tracks/01.mp3",
///     "path": "/data/nft/promo/album123/tracks/01.mp3",
///     "filename": "01.mp3"
///   }
async fn upload_file(
    State(config): State<Arc<AppConfig>>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("✅ Multipart parsing successful");

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
            warn!("❌ Field read error: {:?}", e);
            error_response(StatusCode::BAD_REQUEST, format!("Field read error: {:?}", e))
        })?
    {
        let name = field.name().unwrap_or("").to_string();
        info!("📦 Processing field: {}", name);

        match name.as_str() {
            "file" => {
                original_filename = field.file_name().map(|s| s.to_string());
                info!("📄 File field found: {:?}", original_filename);

                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| {
                        warn!("❌ File bytes read error: {:?}", e);
                        error_response(StatusCode::BAD_REQUEST, format!("File read error: {:?}", e))
                    })?
                    .to_vec();

                info!("✅ File bytes read: {} bytes", bytes.len());
                file_data = Some(bytes);
            }
            "album_id" => {
                let text = field.text().await.map_err(|e| {
                    warn!("❌ album_id read error: {:?}", e);
                    error_response(StatusCode::BAD_REQUEST, format!("album_id error: {:?}", e))
                })?;
                info!("📝 album_id: {}", text);
                album_id = Some(text);
            }
            "file_type" => {
                let text = field.text().await.map_err(|e| {
                    warn!("❌ file_type read error: {:?}", e);
                    error_response(StatusCode::BAD_REQUEST, format!("file_type error: {:?}", e))
                })?;
                info!("📝 file_type: {}", text);
                file_type = Some(text);
            }
            "category" => {
                let text = field.text().await.map_err(|e| {
                    warn!("❌ category read error: {:?}", e);
                    error_response(StatusCode::BAD_REQUEST, format!("category error: {:?}", e))
                })?;
                info!("📝 category: {}", text);
                category = Some(text);
            }
            "track_number" => {
                let text = field.text().await.map_err(|e| {
                    warn!("❌ track_number read error: {:?}", e);
                    error_response(StatusCode::BAD_REQUEST, format!("track_number error: {:?}", e))
                })?;
                info!("📝 track_number: {}", text);
                track_number = Some(text);
            }
            _ => {
                warn!("⚠️  Unknown field: {}", name);
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
    if category != "tracks" && category != "cover" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "category must be 'tracks' or 'cover'".to_string(),
        ));
    }

    // ファイル名の生成
    let extension = original_filename
        .split('.')
        .last()
        .unwrap_or("bin")
        .to_lowercase();

    let filename = if category == "tracks" {
        // tracks の場合は track_number が必須
        let track_num = track_number.ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "track_number is required for tracks".to_string(),
            )
        })?;
        format!("{}.{}", track_num, extension)
    } else {
        // cover の場合
        format!("cover.{}", extension)
    };

    // 保存先ディレクトリの構築
    let target_dir = if category == "tracks" {
        config
            .base_data_dir
            .join(&file_type)
            .join(&album_id)
            .join("tracks")
    } else {
        config.base_data_dir.join(&file_type).join(&album_id)
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

    info!("✅ File saved: {:?}", target_path);

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
            config.vps_base_url, file_type, album_id, filename
        )
    } else {
        format!(
            "{}/{}/{}/{}",
            config.vps_base_url, file_type, album_id, filename
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
    State(config): State<Arc<AppConfig>>,
    Json(payload): Json<DeleteRequest>,
) -> Result<Json<DeleteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let target_dir = config
        .base_data_dir
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

    info!("🗑️  Deleted: {:?}", target_dir);

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

    let config = Arc::new(AppConfig::default());

    // ルーター構築
    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/upload", post(upload_file))
        .route("/api/delete", post(delete_file))
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50MB まで許可
        .layer(CorsLayer::permissive())
        .with_state(config);

    let addr = "0.0.0.0:3000";
    info!("🚀 NFT Upload API Server listening on {}", addr);
    info!("📦 Max body size: 50MB");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
