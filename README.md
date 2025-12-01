# NFT Upload API Server

VPS 側で動作する、NFT ファイルアップロード用の Rust API サーバ

## 機能

- **POST /api/upload**: ファイルアップロード（音源・画像）
- **POST /api/delete**: ファイル削除（売り切れ時）
- **GET /api/health**: ヘルスチェック

## ビルド・実行

### ローカル開発（Windows）

```bash
cd upload_server_rust
cargo build
cargo run
```

### VPS デプロイ

```bash
# VPS に SSH でログイン
ssh ubuntu@153.121.61.17

# プロジェクトをクローン or 転送
cd /srv
sudo mkdir -p toodigital
sudo chown ubuntu:ubuntu toodigital
cd toodigital
git clone <your-repo-url>

# または Windows から SCP
scp -r C:\dev\vps\upload_server_rust ubuntu@153.121.61.17:/srv/toodigital/

# VPS 側でビルド
cd /srv/toodigital/upload_server_rust
cargo build --release

# systemd サービスとして起動
sudo cp upload-api.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable upload-api
sudo systemctl start upload-api
```

## API 仕様

### 1. ヘルスチェック

**Request**:
```
GET http://153.121.61.17:3000/api/health
```

**Response**:
```json
{
  "status": "ok",
  "service": "nft-upload-api",
  "version": "0.1.0"
}
```

### 2. ファイルアップロード

**Request**:
```
POST http://153.121.61.17:3000/api/upload
Content-Type: multipart/form-data

file: <binary>
album_id: "album123"
file_type: "promo" | "albums"
category: "tracks" | "cover"
track_number: "01" (tracks の場合のみ)
```

**Response**:
```json
{
  "success": true,
  "url": "http://153.121.61.17/nft/promo/album123/tracks/01.mp3",
  "path": "/data/nft/promo/album123/tracks/01.mp3",
  "filename": "01.mp3"
}
```

### 3. ファイル削除

**Request**:
```
POST http://153.121.61.17:3000/api/delete
Content-Type: application/json

{
  "album_id": "album123",
  "file_type": "albums"
}
```

**Response**:
```json
{
  "success": true,
  "message": "Deleted \"/data/nft/albums/album123\""
}
```

## ディレクトリ構造

```
/data/nft/
├── promo/
│   └── album123/
│       ├── tracks/
│       │   ├── 01.mp3
│       │   └── 02.mp3
│       └── cover.jpg
└── albums/
    └── album123/
        ├── tracks/
        │   ├── 01.flac
        │   └── 02.flac
        └── cover.jpg
```

## systemd サービス設定

`/etc/systemd/system/upload-api.service`:

```ini
[Unit]
Description=NFT Upload API Server
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/srv/toodigital/upload_server_rust
ExecStart=/srv/toodigital/upload_server_rust/target/release/nft-upload-server
Restart=always
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
```

## セキュリティ

- ファイルアップロード後、所有権を `caddy:caddy` に変更
- CORS を許可（開発用）、本番では特定ドメインのみに制限推奨

## ログ

```bash
# サービスログの確認
sudo journalctl -u upload-api -f
```
