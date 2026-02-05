use axum::{
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::AppState;

const CAMERA_TEMP_DIR: &str = "/data/camera_temp";
const LATEST_FILE: &str = "/data/camera_temp/latest";

/// GET /camera — モバイル向けカメラ撮影ページ
pub async fn camera_page() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Camera Upload</title>
<style>
*{margin:0;padding:0;box-sizing:border-box}
body{background:#111;color:#fff;font-family:-apple-system,sans-serif;
  display:flex;flex-direction:column;align-items:center;
  min-height:100vh;padding:24px}
h1{font-size:20px;margin-bottom:24px;color:#aaa}
.btn{background:#6750A4;color:#fff;border:none;border-radius:12px;
  padding:16px 32px;font-size:18px;cursor:pointer;width:100%;max-width:320px}
.btn:active{background:#7E67C1}
.btn:disabled{background:#333;color:#666}
#preview{max-width:300px;max-height:400px;margin:16px 0;border-radius:12px;display:none}
#status{margin-top:16px;font-size:16px;text-align:center}
.success{color:#4CAF50}
.error{color:#f44336}
.uploading{color:#FF9800}
input[type=file]{display:none}
</style>
</head>
<body>
<h1>TD Image Studio</h1>
<button class="btn" id="captureBtn" onclick="document.getElementById('fileInput').click()">
  カメラで撮影
</button>
<input type="file" id="fileInput" accept="image/*" capture="environment">
<img id="preview">
<div id="status"></div>
<script>
const fileInput=document.getElementById('fileInput');
const preview=document.getElementById('preview');
const status=document.getElementById('status');
const btn=document.getElementById('captureBtn');

fileInput.addEventListener('change',async(e)=>{
  const file=e.target.files[0];
  if(!file)return;

  // Show preview
  const reader=new FileReader();
  reader.onload=(ev)=>{
    preview.src=ev.target.result;
    preview.style.display='block';
  };
  reader.readAsDataURL(file);

  // Upload
  status.className='uploading';
  status.textContent='アップロード中...';
  btn.disabled=true;

  try{
    const form=new FormData();
    form.append('image',file);
    const res=await fetch('/api/camera/upload',{method:'POST',body:form});
    if(res.ok){
      status.className='success';
      status.textContent='アップロード完了！アプリで取得してください。';
    }else{
      const text=await res.text();
      status.className='error';
      status.textContent='エラー: '+text;
    }
  }catch(err){
    status.className='error';
    status.textContent='通信エラー: '+err.message;
  }
  btn.disabled=false;
});
</script>
</body>
</html>"#)
}

/// POST /api/camera/upload — モバイルから画像受信
pub async fn upload_image(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // camera_temp ディレクトリ作成
    fs::create_dir_all(CAMERA_TEMP_DIR).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {}", e))
    })?;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "image" {
            let bytes = field.bytes().await.map_err(|e| {
                (StatusCode::BAD_REQUEST, format!("Read error: {}", e))
            })?;

            info!("Camera upload received: {} bytes", bytes.len());

            let mut file = fs::File::create(LATEST_FILE).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("File create error: {}", e))
            })?;

            file.write_all(&bytes).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Write error: {}", e))
            })?;

            info!("Camera image saved to {}", LATEST_FILE);
            return Ok((StatusCode::OK, "OK"));
        }
    }

    Err((StatusCode::BAD_REQUEST, "No image field found".to_string()))
}

/// GET /api/camera/latest — 最新画像を返す
pub async fn get_latest(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let bytes = fs::read(LATEST_FILE).await.map_err(|_| StatusCode::NOT_FOUND)?;

    // Content-Type を推定 (JPEG/PNG)
    let content_type = if bytes.len() >= 4 && bytes[0..4] == [0x89, 0x50, 0x4E, 0x47] {
        "image/png"
    } else {
        "image/jpeg"
    };

    Ok(([(header::CONTENT_TYPE, content_type)], bytes))
}

/// DELETE /api/camera/latest — 画像削除（クリーンアップ）
pub async fn delete_latest(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    match fs::remove_file(LATEST_FILE).await {
        Ok(_) => {
            info!("Camera temp file deleted");
            Ok((StatusCode::OK, "Deleted"))
        }
        Err(_) => {
            warn!("Camera temp file not found for deletion");
            Ok((StatusCode::OK, "Not found (already clean)"))
        }
    }
}
