# NetDisk

Rust + Axum 网盘后端，使用 Diesel 管理 SQLite 元数据，按 BLAKE3 内容哈希保存文件。提供注册、登录、JWT 认证、文件与文件夹管理、上传、下载和文件夹 ZIP 下载，以及独立的 Vite 前端。

## 启动后端

1. 将 `.env.example` 复制为 `.env`。
2. 设置 `DATABASE_URL`，并使用 `openssl rand -hex 32` 生成随机值填入 `JWT_SECRET`（至少 32 字节）。更换密钥后，已有登录令牌失效。
3. 安装支持 SQLite 的 Diesel CLI，执行 `diesel migration run` 初始化数据库。程序启动时不会自动执行迁移。
4. 执行 `cargo run`，后端监听 `0.0.0.0:3000`。

文件内容保存在用户主目录下的 `.netdisk_store` 中。`.env`、本地数据库及构建产物不提交到仓库。

## 分享数据库与接口

已有数据库执行 `diesel migration run` 升级。新增的 `share_table` 保存六位数字分享码、分享集合 ID（`dic_id`）以及 UTC 创建/过期时间；`share_files` 记录集合中的原文件 ID，不改变原文件的父目录。连接池启用 SQLite 外键，删除分享或源文件时会自动清理对应关联。

登录后调用 `POST /create_share`，携带 `Authorization: Bearer <token>`：

```json
{"file_id_list":["文件或文件夹的 UUID"],"expired_at":null}
```

`expired_at` 为 Unix 时间戳（秒），必须在未来；省略或 `null` 表示永久有效。接口返回 HTTP 201 和 `{"share_code":"012345"}`，重复文件 ID 自动去重，只能分享自己的文件。文件夹分享关联文件夹本身，保留对其当前内容的引用。

数据库层提供有效分享查询、关联文件查询、删除分享和清理过期记录。查询时判断有效期，创建分享时清理过期记录，并通过唯一约束和重试避免分享码冲突。

接收者登录后可通过 `GET /shares/{share_code}` 获取文件列表，通过 `GET /shares/{share_code}/download/{file_id}` 下载分享中的文件或文件夹 ZIP；这两个接口都需要 JWT。未登录的公开提取/下载和分享密码尚未实现。

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
