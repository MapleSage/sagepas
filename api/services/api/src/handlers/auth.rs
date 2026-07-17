use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;
use domain::auth::{Claims, UserRole};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub name: String,
    #[serde(default = "default_role")]
    pub role: UserRole,
}

fn default_role() -> UserRole {
    UserRole::Consumer
}

fn require_dev_local_auth(state: &AppState) -> Result<(), (StatusCode, String)> {
    if state.config.dev_local_auth_enabled {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            "local authentication is disabled".to_string(),
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    #[serde(alias = "username")]
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: Uuid,
    pub role: UserRole,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    #[serde(alias = "refresh_token", alias = "refreshToken")]
    pub token: String,
}

#[derive(Debug, sqlx::FromRow)]
struct LoginRow {
    id: Uuid,
    role: String,
}

/// SHA-256-ish placeholder of password + static salt. Replace with Argon2 before production.
fn hash_password(password: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let salted = format!("sagesure::{password}");
    let mut h = DefaultHasher::new();
    salted.hash(&mut h);
    format!("{:x}{:x}", h.finish(), h.finish())
}

/// Issue an HS256 JWT valid for 24 hours.
fn issue_token(user_id: Uuid, role: UserRole, secret: &str) -> anyhow::Result<String> {
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: user_id,
        role,
        iat: now,
        exp: now + 86_400,
    };
    Ok(encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

/// POST /api/v1/auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    require_dev_local_auth(&state)?;
    let id = Uuid::new_v4();
    let pw_hash = hash_password(&req.password);
    let role_str = req.role.to_string();

    sqlx::query(
        r#"
        INSERT INTO users (id, email, name, password_hash, role)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(id)
    .bind(req.email)
    .bind(req.name)
    .bind(pw_hash)
    .bind(role_str)
    .execute(&**state.db)
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    let token = issue_token(id, req.role, &state.config.jwt_private_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        access_token: token.clone(),
        refresh_token: token,
        user_id: id,
        role: req.role,
    }))
}

/// POST /api/v1/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    require_dev_local_auth(&state)?;
    let pw_hash = hash_password(&req.password);

    let row = sqlx::query_as::<_, LoginRow>(
        r#"SELECT id, role FROM users WHERE email = $1 AND password_hash = $2"#,
    )
    .bind(req.email)
    .bind(pw_hash)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| (StatusCode::UNAUTHORIZED, "invalid credentials".to_string()))?;

    let role: UserRole = row.role.parse().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unknown role in db".to_string(),
        )
    })?;

    let token = issue_token(row.id, role, &state.config.jwt_private_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        access_token: token.clone(),
        refresh_token: token,
        user_id: row.id,
        role,
    }))
}

/// POST /api/v1/auth/refresh
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    require_dev_local_auth(&state)?;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;

    let token_data = decode::<Claims>(
        &req.token,
        &DecodingKey::from_secret(state.config.jwt_private_key.as_bytes()),
        &validation,
    )
    .map_err(|e| (StatusCode::UNAUTHORIZED, format!("invalid token: {e}")))?;

    let claims = token_data.claims;
    let new_token = issue_token(claims.sub, claims.role, &state.config.jwt_private_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        access_token: new_token.clone(),
        refresh_token: new_token,
        user_id: claims.sub,
        role: claims.role,
    }))
}
