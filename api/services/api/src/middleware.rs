use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};
use domain::auth::{AuthSource, AuthenticatedUser, Claims, PasRole};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

use crate::state::AppState;

/// Axum middleware that validates the caller's identity on every request
/// except public paths (health check and local-dev auth bootstrap).
///
/// Two token types are accepted:
/// - RS256: a Microsoft Entra ID access token, verified cryptographically
///   against the tenant's JWKS (issuer, audience, expiry, tenant all checked).
/// - HS256: the local dev-only token, only ever accepted when
///   `dev_local_auth_enabled` is explicitly set — production auth is
///   Entra-only.
///
/// Attached via `axum::middleware::from_fn_with_state`.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let path = req.uri().path().to_string();

    let dev_auth_path = state.config.dev_local_auth_enabled
        && matches!(
            path.as_str(),
            "/api/v1/auth/register" | "/api/v1/auth/login" | "/api/v1/auth/refresh"
        );
    let public_common = path == "/health"
        || path == "/api/v1/health"
        || path.starts_with("/api/v1/health/")
        || dev_auth_path;

    // Public routes — no auth required.
    if public_common {
        return Ok(next.run(req).await);
    }

    // Extract the Authorization: Bearer <token> header.
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "missing Authorization header".to_string(),
            )
        })?;

    let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Authorization must use Bearer scheme".to_string(),
        )
    })?;

    // Only the alg is trusted before verification — it routes to the right
    // validator, it is never used to decide *whether* to verify.
    let header = decode_header(token).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            format!("invalid token header: {e}"),
        )
    })?;

    let user = match header.alg {
        Algorithm::RS256 => {
            let validator = state.entra_validator.as_ref().ok_or_else(|| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Entra authentication is not configured on this server".to_string(),
                )
            })?;
            validator.validate(token).await.map_err(|e| {
                (
                    StatusCode::UNAUTHORIZED,
                    format!("invalid Entra token: {e}"),
                )
            })?
        }
        Algorithm::HS256 => {
            if !state.config.dev_local_auth_enabled {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    "local development authentication is disabled; sign in with Microsoft Entra"
                        .to_string(),
                ));
            }

            let mut validation = Validation::new(Algorithm::HS256);
            validation.validate_exp = true;

            let token_data = decode::<Claims>(
                token,
                &DecodingKey::from_secret(state.config.jwt_private_key.as_bytes()),
                &validation,
            )
            .map_err(|e| (StatusCode::UNAUTHORIZED, format!("invalid token: {e}")))?;

            let claims = token_data.claims;
            AuthenticatedUser {
                oid: claims.sub.to_string(),
                email: None,
                name: None,
                tenant_id: None,
                raw_roles: vec![claims.role.to_string()],
                pas_roles: PasRole::from_user_role(claims.role).into_iter().collect(),
                source: AuthSource::DevLocal,
            }
        }
        other => {
            return Err((
                StatusCode::UNAUTHORIZED,
                format!("unsupported token algorithm {other:?}"),
            ));
        }
    };

    // Attach the validated identity to request extensions for handlers to read.
    req.extensions_mut().insert(user);

    Ok(next.run(req).await)
}
