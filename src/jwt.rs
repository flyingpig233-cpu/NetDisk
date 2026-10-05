use crate::user;
use axum::http;
use serde::{Deserialize, Serialize};

pub fn secret_key() -> &'static str {
    static SECRET_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SECRET_KEY.get_or_init(|| {
        let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
        assert!(secret.len() >= 32, "JWT_SECRET must be at least 32 bytes");
        secret
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
}

pub fn generate_token(
    user: &user::User,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp() as usize;
    let exp = now + 86400; // Token valid for 24 hour

    let claims = Claims {
        sub: user.user_id.to_string(),
        exp,
        iat: now,
    };

    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret_key().as_bytes()),
    )
}

pub async fn jwt_auth_middleware(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, http::StatusCode> {
    use axum::http::StatusCode;

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());


    if let Some(auth_header) = auth_header {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            match verify_token(token, secret_key().as_bytes()) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
                Err(_) => {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}


pub fn verify_token(token: &str, secret: &[u8]) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = jsonwebtoken::decode::<Claims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret),
        &jsonwebtoken::Validation::default(),
    )?;
    Ok(token_data.claims)
}
