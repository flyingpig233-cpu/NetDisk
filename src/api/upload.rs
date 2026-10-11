use super::ApiError;
use axum::{
    extract::multipart::{Field, MultipartError},
    http::StatusCode,
};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

// Keep the cleanup guard alive across every await, including request cancellation.
pub(super) struct TemporaryUpload {
    path: PathBuf,
    file: Option<tokio::fs::File>,
}

impl TemporaryUpload {
    fn create(store: &Path) -> io::Result<Self> {
        let path = store.join(format!(".upload-{}", Uuid::new_v4()));
        // Synchronous creation ensures cancellation cannot create an unguarded file.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            path,
            file: Some(tokio::fs::File::from_std(file)),
        })
    }

    pub(super) fn publish(&self, destination: &Path) -> io::Result<()> {
        // Both paths are in the content directory. Publish without overwriting a
        // blob another concurrent upload may have just created.
        match std::fs::hard_link(&self.path, destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for TemporaryUpload {
    fn drop(&mut self) {
        drop(self.file.take());
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(path = %self.path.display(), %error, "Could not remove temporary upload");
            }
        }
    }
}

pub(super) fn multipart_error(error: MultipartError) -> ApiError {
    ApiError {
        status: error.status(),
        message: error.body_text(),
    }
}

pub(super) async fn receive(
    mut field: Field<'_>,
    store: &Path,
    limit: u64,
) -> Result<(TemporaryUpload, String, u64), ApiError> {
    let mut temporary = TemporaryUpload::create(store)?;
    let file = temporary.file.as_mut().unwrap();
    let mut hasher = blake3::Hasher::new();
    let mut size = 0_u64;
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        if chunk.len() as u64 > limit - size {
            return Err(ApiError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("File exceeds the {limit}-byte upload limit"),
            });
        }
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        size += chunk.len() as u64;
    }
    // Tokio writes can still be pending after write_all; finish them before publish.
    file.flush().await?;
    drop(temporary.file.take());
    Ok((temporary, hasher.finalize().to_hex().to_string(), size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::{FromRequest, Multipart},
        http::Request,
    };
    use outdir_tempdir::TempDir;

    fn store() -> TempDir {
        TempDir::builder()
            .platform_temp_dir()
            .build()
            .unwrap()
            .autorm()
    }

    async fn multipart(body: Body) -> Multipart {
        Multipart::from_request(
            Request::builder()
                .header("content-type", "multipart/form-data; boundary=test")
                .body(body)
                .unwrap(),
            &(),
        )
        .await
        .unwrap()
    }

    fn payload(content: &str) -> Body {
        Body::from(format!(
            "--test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\n\r\n{content}\r\n--test--\r\n"
        ))
    }

    #[tokio::test]
    async fn exact_limit_upload_is_hashed_and_published_without_overwriting() {
        let store = store();
        let mut multipart = multipart(payload("hello")).await;
        let field = multipart.next_field().await.unwrap().unwrap();
        let (temporary, hash, size) = receive(field, store.path(), 5).await.unwrap();
        assert_eq!(size, 5);
        assert_eq!(hash, blake3::hash(b"hello").to_hex().to_string());
        let destination = store.path().join(hash);
        temporary.publish(&destination).unwrap();
        let original = std::fs::metadata(&destination).unwrap().modified().unwrap();
        temporary.publish(&destination).unwrap();
        assert_eq!(
            std::fs::metadata(&destination).unwrap().modified().unwrap(),
            original
        );
        drop(temporary);
        assert_eq!(std::fs::read(destination).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(store.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn oversized_upload_returns_413_and_removes_temporary_file() {
        let store = store();
        let mut multipart = multipart(payload("hello!")).await;
        let field = multipart.next_field().await.unwrap().unwrap();
        let error = receive(field, store.path(), 5).await.err().unwrap();
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(std::fs::read_dir(store.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn broken_upload_returns_400_and_removes_temporary_file() {
        let store = store();
        let mut multipart = multipart(Body::from(
            "--test\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nhello",
        ))
        .await;
        let field = multipart.next_field().await.unwrap().unwrap();
        let error = receive(field, store.path(), 100).await.err().unwrap();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_dir(store.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn cancelling_pending_upload_removes_temporary_file() {
        let store = store();
        let (mut sender, receiver) = tokio::io::duplex(1024);
        sender
            .write_all(b"--test\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nhello")
            .await
            .unwrap();
        let mut multipart = multipart(Body::from_stream(tokio_util::io::ReaderStream::new(
            receiver,
        )))
        .await;
        let field = multipart.next_field().await.unwrap().unwrap();
        let mut future = Box::pin(receive(field, store.path(), 100));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut future)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(store.path()).unwrap().count(), 1);
        drop(future);
        assert_eq!(std::fs::read_dir(store.path()).unwrap().count(), 0);
    }
}
