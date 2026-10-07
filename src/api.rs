use std::{fs::File, str::FromStr};

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
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
        .route(
            "/files",
            get(list_files).post(create_file).delete(delete_file),
        )
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
        .route("/upload", post(upload).layer(DefaultBodyLimit::disable()))
        .route("/move", post(move_file))
        .route("/user_info", get(get_user_info))
        .route("/download/{file_id}", get(download))
        .layer(middleware::from_fn(jwt_auth_middleware));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
}

async fn root() -> &'static str {
    "Hello, World!"
}

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
        let status = if matches!(
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
}

async fn list_files(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<FileMeta>>, ApiError> {
    let parent_id = match query.parent_id.as_deref() {
        None | Some("") => ROOT_ID,
        Some(value) => Uuid::parse_str(value).map_err(|_| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid parent ID".to_string(),
        })?,
    };
    let mut fm = state.manager()?;
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
    let user = UserManager::create_user(&mut conn, req.username, &req.password)?;
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
    Extension(claims): Extension<crate::jwt::Claims>,
    Json(req): Json<MoveFileRequest>,
) -> Result<StatusCode, ApiError> {
    let owner = Uuid::from_str(&claims.sub).map_err(|_| ApiError::internal("Invalid user ID"))?;
    let mut fm = state.manager()?;
    let file_meta = fm.get_file_meta(req.file_id)?.ok_or(FileError::NotFound)?;
    if file_meta.file_owner != owner {
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
        if parent.file_owner != owner {
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
        .ok_or_else(|| ApiError::internal("User not found"))?;

    if !user.verify_password(&req.password) {
        return Err(ApiError::internal("Invalid password"));
    }

    let token = crate::jwt::generate_token(&user)
        .map_err(|e| ApiError::internal(format!("Token generation error: {e}")))?;

    Ok((StatusCode::OK, token))
}

async fn upload(
    State(state): State<AppState>,
    Extension(claims): Extension<crate::jwt::Claims>,
    Query(query): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<FileMeta>), ApiError> {
    let owner = Uuid::from_str(&claims.sub).map_err(|_| ApiError::internal("Invalid user ID"))?;
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
    raw_download(State(state), target_file).await
}

async fn download(
    State(state): State<AppState>,
    Extension(claims): Extension<crate::jwt::Claims>,
    Path(file_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let mut fm = state.manager()?;
    let meta = fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?;
    if meta.file_owner
        != Uuid::from_str(&claims.sub).map_err(|_| ApiError::internal("Invalid user ID"))?
    {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "Cannot download another user's file".to_string(),
        });
    }
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

async fn create_share(
    State(state): State<AppState>,
    Extension(claims): Extension<crate::jwt::Claims>,
    Json(req): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let owner = Uuid::from_str(&claims.sub).map_err(|_| ApiError::internal("Invalid user ID"))?;
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let share = ShareManager::create_share(&mut conn, owner, req.file_id_list, req.expired_at)?;
    Ok((
        StatusCode::CREATED,
        Json(ShareResponse {
            share_code: share.share_code,
        }),
    ))
}

async fn get_file(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
) -> Result<Json<FileMeta>, ApiError> {
    let mut fm = state.manager()?;
    Ok(Json(fm.get_file_meta(file_id)?.ok_or(FileError::NotFound)?))
}

async fn get_user_info(
    State(state): State<AppState>,
    Extension(claims): Extension<crate::jwt::Claims>,
) -> Result<Json<User>, ApiError> {
    let mut conn = state
        .db
        .get()
        .map_err(|e| ApiError::internal(format!("database pool error: {e}")))?;
    let user = UserManager::get_user_by_id(
        &mut conn,
        Uuid::from_str(&claims.sub).map_err(|_| ApiError::internal("Invalid user ID"))?,
    )?
    .ok_or_else(|| ApiError::internal("User not found"))?;
    Ok(Json(user))
}

async fn create_file(
    State(state): State<AppState>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<FileMeta>), ApiError> {
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

    let mut fm = state.manager()?;
    let _ = fm.new_file(meta.clone())?;

    Ok((StatusCode::CREATED, Json(meta)))
}
async fn delete_file(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut fm = state.manager()?;
    fm.delete_file(file_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_file(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
    Json(req): Json<RenameRequest>,
) -> Result<StatusCode, ApiError> {
    let mut fm = state.manager()?;
    fm.rename_file(file_id, req.file_name)?;
    Ok(StatusCode::OK)
}
