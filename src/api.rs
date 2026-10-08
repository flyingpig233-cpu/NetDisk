use std::fs::File;

use crate::db_types::UuidSql;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use diesel::prelude::*;
use outdir_tempdir::TempDir;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use zip::ZipWriter;

use crate::share::{ShareError, ShareManager};
use crate::user::User;
use crate::{
    db::DbPool,
    file_system::file_manager::{FileError, FileManager, ROOT_ID, get_store_dir},
    user::UserManager,
};
use crate::{file_system::file_meta::FileMeta, jwt::jwt_auth_middleware};

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
}

pub fn router(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/register", post(register_user))
        .route("/login", post(login_user));
    let protected_routes = Router::new()
        .route("/", get(root))
        .route("/files", get(list_files).post(create_file))
        .route(
            "/files/{file_id}",
            get(get_file).delete(delete_file).patch(rename_file),
        )
        .route("/shares/{share_code}", get(get_share))
        .route(
            "/shares/{share_code}/download/{file_id}",
            get(download_share),
        )
        .route("/create_share", post(create_share))
        .route("/share_records", get(list_share_records))
        .route(
            "/share_records/{share_id}",
            axum::routing::delete(revoke_share),
        )
        .route("/upload", post(upload).layer(DefaultBodyLimit::disable()))
        .route("/move", post(move_file))
        .route("/user_info", get(get_user_info))
        .route("/admin/users", get(list_users))
        .route(
            "/admin/users/{user_id}",
            get(admin_get_user).patch(update_user).delete(delete_user),
        )
        .route("/download/{file_id}", get(download))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            jwt_auth_middleware,
        ));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
}

async fn root() -> &'static str {
    "Hello, World!"
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl From<diesel::result::Error> for ApiError {
    fn from(value: diesel::result::Error) -> Self {
        let status = if matches!(&value, diesel::result::Error::NotFound) {
            StatusCode::NOT_FOUND
        } else if matches!(
            &value,
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _
            )
        ) {
            StatusCode::CONFLICT
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError {
            status,
            message: value.to_string(),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        ApiError::internal(value.to_string())
    }
}

impl From<ShareError> for ApiError {
    fn from(value: ShareError) -> Self {
        let (status, message) = match value {
            ShareError::EmptyFiles => (
                StatusCode::BAD_REQUEST,
                "Share must contain at least one file",
            ),
            ShareError::InvalidExpiration => (
                StatusCode::BAD_REQUEST,
                "Expiration must be a valid future Unix timestamp",
            ),
            ShareError::NotFound => (StatusCode::NOT_FOUND, "File not found"),
            ShareError::Forbidden => (StatusCode::FORBIDDEN, "Cannot share another user's file"),
            ShareError::CodeExhausted => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Could not allocate a share code; try again",
            ),
            ShareError::Database(error) => return error.into(),
        };
        Self {
            status,
            message: message.to_string(),
        }
    }
}

impl From<FileError> for ApiError {
    fn from(value: FileError) -> Self {
        let status = match &value {
            FileError::NotFound => StatusCode::NOT_FOUND,
            FileError::InvalidMove => StatusCode::BAD_REQUEST,
            FileError::Database(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => StatusCode::CONFLICT,
            FileError::Database(_) | FileError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError {
            status,
            message: value.to_string(),
        }
    }
}

impl From<zip::result::ZipError> for ApiError {
    fn from(value: zip::result::ZipError) -> Self {
        ApiError::internal(value.to_string())
    }
}

impl AppState {
    fn manager(&self) -> Result<FileManager, ApiError> {
        let conn = self
            .db
            .get()
            .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
        Ok(FileManager::new(conn))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

#[derive(Deserialize)]
struct ListQuery {
    user_id: Uuid,
    parent_id: Option<String>,
}

#[derive(Deserialize)]
struct CreateFileRequest {
    file_name: String,
    #[serde(default)]
    file_size: u64,
    file_hash: String,
    file_owner: Uuid,
    #[serde(default)]
    parent_id: Option<Uuid>,
    #[serde(default)]
    is_directory: bool,
}

#[derive(Deserialize)]
struct RenameRequest {
    file_name: String,
}

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct MoveFileRequest {
    file_id: Uuid,
    new_parent_id: Option<String>,
}

#[derive(Deserialize)]
struct CreateShareRequest {
    file_id_list: Vec<Uuid>,
    expired_at: Option<i64>,
}

#[derive(serde::Serialize)]
struct ShareResponse {
    share_code: String,
}

#[derive(Deserialize)]
struct UploadQuery {
    parent_id: Option<String>,
    user_id: Option<Uuid>,
}

async fn list_files(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<FileMeta>>, ApiError> {
    let parent_id = match query.parent_id.as_deref() {
        None | Some("") => ROOT_ID,
        Some(value) => Uuid::parse_str(value).map_err(|_| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid parent ID".to_string(),
        })?,
    };
    authorize_owner(&actor, query.user_id)?;
    let mut fm = state.manager()?;
    validate_parent(&mut fm, parent_id, query.user_id)?;
    Ok(Json(fm.get_file_list(query.user_id, parent_id)?))
}

async fn register_user(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<User>), ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let name = validate_username(&req.username)?;
    if name.eq_ignore_ascii_case("admin") {
        return Err(bad_request("admin is a reserved username"));
    }
    if req.password.is_empty() {
        return Err(bad_request("Password cannot be empty"));
    }
    let user = UserManager::create_user(&mut conn, name, &req.password)?;
    Ok((StatusCode::CREATED, Json(user)))
}

async fn get_share(
    State(state): State<AppState>,
    Path(share_code): Path<String>,
) -> Result<Json<Vec<FileMeta>>, ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let file_list = ShareManager::get_files(&mut conn, &share_code)?;
    Ok(Json(file_list))
}

async fn move_file(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Json(req): Json<MoveFileRequest>,
) -> Result<StatusCode, ApiError> {
    let owner = actor.user_id;
    let mut fm = state.manager()?;
    let file_meta = fm.get_file_meta(req.file_id)?.ok_or(FileError::NotFound)?;
    if file_meta.file_owner != owner && !actor.is_admin {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "Cannot move another user's file".to_string(),
        });
    }
    let new_parent_id = match req.new_parent_id.as_deref() {
        None | Some("") => ROOT_ID,
        Some(value) => Uuid::parse_str(value).map_err(|_| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid new parent ID".to_string(),
        })?,
    };
    if new_parent_id != ROOT_ID {
        let parent = fm
            .get_file_meta(new_parent_id)?
            .ok_or(FileError::NotFound)?;
        if parent.file_owner != file_meta.file_owner {
            return Err(ApiError {
                status: StatusCode::FORBIDDEN,
                message: "Cannot move to another user's folder".to_string(),
            });
        }
        if !parent.is_directory {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "New parent must be a directory".to_string(),
            });
        }
    }
    if req.file_id == ROOT_ID {
        return Err(bad_request("Cannot move the root directory"));
    }
    fm.move_file(req.file_id, Some(new_parent_id))?;
    Ok(StatusCode::OK)
}

async fn login_user(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, String), ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let user = UserManager::get_user_by_username(&mut conn, &req.username)?
        .ok_or_else(|| unauthorized())?;

    if !user.verify_password(&req.password) {
        return Err(unauthorized());
    }

    let token = crate::jwt::generate_token(&user)
        .map_err(|e| ApiError::internal(format!("Token generation error: {e}")))?;

    Ok((StatusCode::OK, token))
}

async fn upload(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Query(query): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<FileMeta>), ApiError> {
    let owner = query.user_id.unwrap_or(actor.user_id);
    authorize_owner(&actor, owner)?;
    ensure_user_exists(&state, owner)?;
    let parent_id = match query.parent_id.as_deref() {
        None | Some("") => ROOT_ID,
        Some(value) => Uuid::parse_str(value).map_err(|_| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid parent ID".to_string(),
        })?,
    };
    if parent_id != ROOT_ID {
        let mut fm = state.manager()?;
        let parent = fm.get_file_meta(parent_id)?.ok_or(FileError::NotFound)?;
        if parent.file_owner != owner {
            return Err(ApiError {
                status: StatusCode::FORBIDDEN,
                message: "Cannot upload to another user's folder".to_string(),
            });
        }
        if !parent.is_directory {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "Parent must be a directory".to_string(),
            });
        }
    }

    let mut field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("multipart error: {e}")))?
        .ok_or_else(|| ApiError::internal("missing file field"))?;
    let file_name = field.file_name().unwrap_or("unnamed").to_string();
    validate_file_name(&file_name)?;

    let store = get_store_dir();
    let tmp_path = store.join(format!(".upload-{}", Uuid::new_v4()));
    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut size: u64 = 0;

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| ApiError::internal(format!("multipart error: {e}")))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        size += chunk.len() as u64;
    }
    drop(file);

    let hash = hasher.finalize().to_hex().to_string();
    let path = store.join(&hash);
    if path.exists() {
        tokio::fs::remove_file(&tmp_path).await?;
    } else {
        tokio::fs::rename(&tmp_path, &path).await?;
    }

    let now = chrono::Utc::now().timestamp() as u32;
    let meta = FileMeta {
        file_id: Uuid::new_v4(),
        file_name,
        file_size: size,
        file_hash: hash,
        file_owner: owner,
        file_created_at: now,
        file_updated_at: now,
        parent_id,
        is_directory: false,
    };

    let mut fm = state.manager()?;
    fm.new_file(meta.clone())?;
    Ok((StatusCode::CREATED, Json(meta)))
}

async fn download_share(
    State(state): State<AppState>,
    Path((share_code, file_id)): Path<(String, Uuid)>,
) -> Result<Response, ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let file_list = ShareManager::get_files(&mut conn, &share_code)?;
    let target_file = file_list
        .into_iter()
        .find(|file| file.file_id == file_id)
        .ok_or(ShareError::NotFound)?;
    drop(conn);
    raw_download(State(state), target_file).await
}

async fn download(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(file_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let mut fm = state.manager()?;
    let meta = fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?;
    authorize_owner(&actor, meta.file_owner)?;
    drop(fm);
    raw_download(State(state), meta).await
}

async fn raw_download(State(state): State<AppState>, meta: FileMeta) -> Result<Response, ApiError> {
    let mut fm = state.manager()?;
    if meta.is_directory {
        let dir = TempDir::new().autorm();
        let zip_list = fm.list_dict_recursive(meta.file_id, String::new())?;
        println!("zip_list: {:?}", zip_list);
        let zip_path = dir.path().join(format!("{}.zip", meta.file_hash));
        let zip_file = File::create(&zip_path)?;
        let options = zip::write::FileOptions::<()>::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut zip_writer = ZipWriter::new(zip_file);
        for zip_file in zip_list {
            let mut file = File::open(zip_file.real_path)?;
            zip_writer.start_file(zip_file.zip_path, options)?;
            std::io::copy(&mut file, &mut zip_writer)?;
        }
        zip_writer.finish()?;
        let zip_size = std::fs::metadata(zip_path.clone())?.len();
        let stream = ReaderStream::new(tokio::fs::File::open(zip_path).await?);
        let body = axum::body::Body::from_stream(stream);
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Length", zip_size.to_string())
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{}.zip\"", meta.file_name),
            )
            .body(body)
            .unwrap());
    }

    let store = get_store_dir();
    let path = store.join(&meta.file_hash);
    if !path.exists() {
        return Err(FileError::NotFound.into());
    }
    let stream = ReaderStream::new(tokio::fs::File::open(path).await?);
    let body = axum::body::Body::from_stream(stream);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Length", meta.file_size.to_string())
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", meta.file_name),
        )
        .body(body)
        .unwrap())
}

#[derive(Deserialize)]
struct ShareListQuery {
    user_id: Option<Uuid>,
    #[serde(default)]
    all: bool,
}
async fn list_share_records(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Query(query): Query<ShareListQuery>,
) -> Result<Json<Vec<crate::share::ShareRecord>>, ApiError> {
    let owner = if query.all {
        require_admin(&actor)?;
        None
    } else {
        let id = query.user_id.unwrap_or(actor.user_id);
        authorize_owner(&actor, id)?;
        ensure_user_exists(&state, id)?;
        Some(id)
    };
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(ShareManager::list_records(&mut conn, owner)?))
}
async fn revoke_share(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    ShareManager::revoke(&mut conn, id, actor.user_id, actor.is_admin)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_share(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Json(req): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let owner = actor.user_id;
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let share = ShareManager::create_share_as(
        &mut conn,
        owner,
        req.file_id_list,
        req.expired_at,
        actor.is_admin,
    )?;
    Ok((
        StatusCode::CREATED,
        Json(ShareResponse {
            share_code: share.share_code,
        }),
    ))
}

async fn get_file(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(file_id): Path<Uuid>,
) -> Result<Json<FileMeta>, ApiError> {
    let mut fm = state.manager()?;
    let meta = fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?;
    authorize_owner(&actor, meta.file_owner)?;
    Ok(Json(meta))
}

async fn get_user_info(Extension(actor): Extension<User>) -> Json<User> {
    Json(actor)
}

async fn create_file(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<FileMeta>), ApiError> {
    authorize_owner(&actor, req.file_owner)?;
    ensure_user_exists(&state, req.file_owner)?;
    validate_file_name(&req.file_name)?;
    if !req.is_directory || !req.file_hash.is_empty() || req.file_size != 0 {
        return Err(bad_request(
            "Use /upload for file contents; /files only creates directories",
        ));
    }
    let mut fm = state.manager()?;
    validate_parent(&mut fm, req.parent_id.unwrap_or(ROOT_ID), req.file_owner)?;
    let now = chrono::Utc::now().timestamp() as u32;
    let meta = FileMeta {
        file_id: Uuid::new_v4(),
        file_name: req.file_name,
        file_size: req.file_size,
        file_hash: req.file_hash,
        file_owner: req.file_owner,
        file_created_at: now,
        file_updated_at: now,
        parent_id: req.parent_id.unwrap_or(Uuid::nil()),
        is_directory: req.is_directory,
    };

    fm.new_file(meta.clone())?;

    Ok((StatusCode::CREATED, Json(meta)))
}
async fn delete_file(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(file_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut fm = state.manager()?;
    let meta = fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?;
    authorize_owner(&actor, meta.file_owner)?;
    if file_id == ROOT_ID {
        return Err(bad_request("Cannot delete the root directory"));
    }
    fm.delete_file(file_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_file(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
    Extension(actor): Extension<User>,
    Json(req): Json<RenameRequest>,
) -> Result<StatusCode, ApiError> {
    let mut fm = state.manager()?;
    let meta = fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?;
    authorize_owner(&actor, meta.file_owner)?;
    if file_id == ROOT_ID {
        return Err(bad_request("Cannot rename the root directory"));
    }
    validate_file_name(&req.file_name)?;
    fm.rename_file(file_id, req.file_name)?;
    Ok(StatusCode::OK)
}

fn bad_request(message: &str) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}
fn unauthorized() -> ApiError {
    ApiError {
        status: StatusCode::UNAUTHORIZED,
        message: "Invalid username or password".into(),
    }
}
fn authorize_owner(actor: &User, owner: Uuid) -> Result<(), ApiError> {
    if actor.is_admin || actor.user_id == owner {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "Access denied".into(),
        })
    }
}
fn require_admin(actor: &User) -> Result<(), ApiError> {
    if actor.is_admin {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "Administrator access required".into(),
        })
    }
}
fn ensure_user_exists(state: &AppState, id: Uuid) -> Result<(), ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    UserManager::get_user_by_id(&mut conn, id)?.ok_or(diesel::result::Error::NotFound)?;
    Ok(())
}
fn validate_parent(fm: &mut FileManager, parent_id: Uuid, owner: Uuid) -> Result<(), ApiError> {
    if parent_id == ROOT_ID {
        return Ok(());
    }
    let parent = fm.get_file_meta(parent_id)?.ok_or(FileError::NotFound)?;
    if !parent.is_directory || parent.file_owner != owner {
        return Err(bad_request(
            "Parent must be a directory owned by the file owner",
        ));
    }
    Ok(())
}
fn validate_username(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(bad_request("Username must contain 1 to 64 characters"));
    }
    Ok(name.into())
}
fn validate_file_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > 255
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err(bad_request("Invalid file name"));
    }
    Ok(())
}
async fn list_users(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
) -> Result<Json<Vec<User>>, ApiError> {
    require_admin(&actor)?;
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(
        crate::schema::users::table
            .order(crate::schema::users::username.asc())
            .load(&mut conn)?,
    ))
}
async fn admin_get_user(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(id): Path<Uuid>,
) -> Result<Json<User>, ApiError> {
    require_admin(&actor)?;
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(
        UserManager::get_user_by_id(&mut conn, id)?.ok_or(diesel::result::Error::NotFound)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateUserRequest {
    username: Option<String>,
    password: Option<String>,
    is_admin: Option<bool>,
}
async fn update_user(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<User>, ApiError> {
    require_admin(&actor)?;
    use crate::schema::users::dsl::*;
    let name = req.username.as_deref().map(validate_username).transpose()?;
    if req.password.as_ref().is_some_and(|p| p.len() < 8) {
        return Err(bad_request("New password must be at least 8 bytes"));
    }
    let hash = req
        .password
        .as_deref()
        .map(crate::user::hash_password)
        .transpose()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let changed = conn.immediate_transaction::<_, ApiError, _>(|conn| {
        let mut target =
            UserManager::get_user_by_id(conn, id)?.ok_or(diesel::result::Error::NotFound)?;
        if target.is_admin
            && req.is_admin == Some(false)
            && users
                .filter(is_admin.eq(true))
                .count()
                .get_result::<i64>(conn)?
                <= 1
        {
            return Err(bad_request("Cannot demote the last administrator"));
        }
        if let Some(value) = &name {
            target.username = value.clone();
        }
        if let Some(value) = &hash {
            target.password_hash = value.clone();
            target.token_version += 1;
        }
        if let Some(value) = req.is_admin {
            target.is_admin = value;
        }
        target.updated_at = chrono::Utc::now().timestamp() as u32;
        diesel::update(users.find(UuidSql::from(id)))
            .set((
                username.eq(&target.username),
                password_hash.eq(&target.password_hash),
                is_admin.eq(target.is_admin),
                token_version.eq(target.token_version),
                updated_at.eq(i64::from(target.updated_at)),
            ))
            .execute(conn)?;
        Ok(target)
    })?;
    Ok(Json(changed))
}

/// Delete metadata atomically; blob cleanup is best effort after the account is gone.
fn delete_user_records(
    conn: &mut SqliteConnection,
    actor: &User,
    id: Uuid,
) -> Result<Vec<String>, ApiError> {
    require_admin(actor)?;
    if actor.user_id == id {
        return Err(bad_request("Cannot delete the currently logged-in account"));
    }
    use crate::schema::{file_meta, share_files, share_table, users};
    conn.immediate_transaction(|conn| {
        let target =
            UserManager::get_user_by_id(conn, id)?.ok_or(diesel::result::Error::NotFound)?;
        if target.is_admin
            && users::table
                .filter(users::is_admin.eq(true))
                .count()
                .get_result::<i64>(conn)?
                <= 1
        {
            return Err(bad_request("Cannot delete the last administrator"));
        }
        let memberships = share_files::table
            .inner_join(file_meta::table)
            .filter(file_meta::file_owner.eq(UuidSql::from(id)))
            .select(share_files::dic_id)
            .distinct()
            .load::<String>(conn)?;
        let hashes = file_meta::table
            .filter(file_meta::file_owner.eq(UuidSql::from(id)))
            .filter(file_meta::is_directory.eq(false))
            .select(file_meta::file_hash)
            .distinct()
            .load::<String>(conn)?;
        diesel::delete(file_meta::table.filter(file_meta::file_owner.eq(UuidSql::from(id))))
            .execute(conn)?;
        // Memberships cascade with file deletion. Remove only affected shares that are now empty.
        diesel::delete(
            share_table::table
                .filter(share_table::dic_id.eq_any(memberships))
                .filter(diesel::dsl::not(
                    share_table::dic_id.eq_any(share_files::table.select(share_files::dic_id)),
                )),
        )
        .execute(conn)?;
        diesel::delete(share_table::table.filter(share_table::owner_id.eq(id.to_string())))
            .execute(conn)?;
        diesel::delete(users::table.find(UuidSql::from(id))).execute(conn)?;
        Ok(hashes)
    })
}
fn cleanup_user_blobs(conn: &mut SqliteConnection, hashes: &[String], store: &std::path::Path) {
    use crate::schema::file_meta;
    for hash in hashes {
        // Only content-addressed names may become filesystem paths.
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let result = conn.immediate_transaction::<_, FileError, _>(|conn| {
            let count = file_meta::table
                .filter(file_meta::file_hash.eq(hash))
                .count()
                .get_result::<i64>(conn)?;
            if count == 0 {
                match std::fs::remove_file(store.join(hash)) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!(%error, "Account deleted; unreferenced content cleanup failed");
        }
    }
}
async fn delete_user(
    State(state): State<AppState>,
    Extension(actor): Extension<User>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let hashes = delete_user_records(&mut conn, &actor, id)?;
    if !hashes.is_empty() {
        cleanup_user_blobs(&mut conn, &hashes, &get_store_dir());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
