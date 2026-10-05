# NetDisk

Rust + Axum 网盘后端，使用 Diesel 管理 SQLite 元数据，按 BLAKE3 内容哈希保存文件。提供注册、登录、JWT 认证、文件与文件夹管理、上传、下载和文件夹 ZIP 下载，以及独立的 Vite 前端。

## 启动后端

1. 将 `.env.example` 复制为 `.env`。
2. 设置 `DATABASE_URL`，并使用 `openssl rand -hex 32` 生成随机值填入 `JWT_SECRET`（至少 32 字节）。更换密钥后，已有登录令牌失效。
3. 安装支持 SQLite 的 Diesel CLI，执行 `diesel migration run` 初始化数据库。程序启动时不会自动执行迁移。
4. 执行 `cargo run`，后端监听 `0.0.0.0:3000`。

文件内容保存在用户主目录下的 `.netdisk_store` 中。`.env`、本地数据库及构建产物不提交到仓库。

## 启动前端

```bash
cd frontend
npm ci
npm run dev
```

访问 http://localhost:5173；开发服务器将 `/api` 请求代理到后端。详细操作和部署说明见 [前端 README](frontend/README.md)。

## 构建检查

```bash
cargo check --locked
cd frontend
npm run build
```
