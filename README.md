# NetDisk

Rust + Axum 网盘后端，使用 Diesel 管理 SQLite 元数据，按 BLAKE3 内容哈希保存文件。提供注册、登录、JWT 认证、文件与文件夹管理、上传、下载和文件夹 ZIP 下载，以及独立的 Vite 前端。

## 启动后端

1. 将 `.env.example` 复制为 `.env`。
2. 设置 `DATABASE_URL`，并使用 `openssl rand -hex 32` 生成随机值填入 `JWT_SECRET`（至少 32 字节）。更换密钥后，已有登录令牌失效。
3. 安装支持 SQLite 的 Diesel CLI，执行 `diesel migration run` 初始化数据库。程序启动时不会自动执行迁移。
4. 执行 `cargo run`，后端监听 `0.0.0.0:3000`。

文件内容保存在用户主目录下的 `.netdisk_store` 中。`.env`、本地数据库及构建产物不提交到仓库。

## 管理员账户

已有数据库先执行 `diesel migration run`。在本地 `.env` 设置 `ADMIN_PASSWORD`（至少 8 字节），再启动后端；当系统没有管理员时，首次启动创建用户名为 `admin` 的管理员账户。也可执行 `cargo run -- --init-admin` 仅初始化账户，不启动 HTTP 服务。已有管理员不会被重新创建，密码不会在重启时覆盖；同名普通账户不会被自动提升权限。

管理员登录后，侧栏出现「用户管理」：查看全部用户 ID、用户名、角色、创建和更新时间，修改用户名、重置密码或调整管理员权限。密码及密码哈希不通过接口返回。重置密码会使该账户已有登录令牌失效，系统禁止取消最后一位管理员的权限。

点击用户的「管理文件」进入其文件空间，可浏览、下载、上传、新建文件夹、重命名、移动、删除和分享文件。界面会显示当前管理的用户，支持返回自己的空间。移动文件保留原所有者，只允许在该所有者的目录之间移动。

- `GET /admin/users`：全部用户列表，仅管理员。
- `GET /admin/users/{user_id}`：用户信息，仅管理员。
- `DELETE /admin/users/{user_id}`：永久删除用户及其全部文件，清理分享关联；共享内容仍被其他用户引用时保留。禁止删除当前登录账号和最后一位管理员。前端要求输入用户名确认。
- `PATCH /admin/users/{user_id}`：修改 `username`、`password`、`is_admin`，均为可选字段，仅管理员。
- 原有文件接口统一验证所有者；管理员可操作任意用户文件。上传时可传 `user_id` 指定所有者，普通用户仅能指定自己。
- `GET /user_info` 返回 `is_admin`。权限以每次请求的数据库角色为准，客户端和 JWT 均不能指定管理员身份。公开注册只能创建普通用户，`admin` 为保留用户名。

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

## 分享列表管理

执行 `diesel migration run` 升级分享创建者字段。用户的「我的分享」从服务器加载，支持按分享码或文件名搜索、查看有效期、复制链接和撤销分享。撤销仅删除分享与关联，保留原文件；接收者之后不能提取或下载该分享。

- `GET /share_records`：当前用户创建的分享，包括数据库中尚未清理的过期记录。
- `GET /share_records?user_id=UUID`：指定用户的分享，仅本人或管理员可读。
- `GET /share_records?all=true`：全部分享，仅管理员。
- `DELETE /share_records/{share_id}`：按稳定的分享集合 UUID 撤销分享，仅创建者或管理员。使用 UUID 避免六位分享码回收后误撤销新分享。

管理员在「用户管理」点击「管理分享」，也可点击「全部分享」管理全站分享。新分享保存实际创建者（管理员分享其他用户的文件时，创建者仍是该管理员）。旧分享没有创建者记录；迁移仅在源文件具有唯一所有者时归到该用户，无法判定的分享由管理员在全部分享中管理。创建新分享时仍会清理过期记录，所以列表并非永久审计历史。
