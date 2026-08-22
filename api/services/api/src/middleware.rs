use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};
use domain::auth::{AuthSource, AuthenticatedUser, Claims, PasRole};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

use crate::state::AppState;

/// Pure path-matching logic, separated from the middleware fn so it can be
/// unit tested without spinning up a router/AppState.
fn is_public_path(path: &str, dev_local_auth_enabled: bool) -> bool {
    let dev_auth_path = dev_local_auth_enabled
        && matches!(
            path,
            "/api/v1/auth/register" | "/api/v1/auth/login" | "/api/v1/auth/refresh"
        );
    path == "/health"
        || path == "/api/v1/health"
        || path.starts_with("/api/v1/health/")
        // Anonymous pricing is deliberate -- the rating handler itself takes
        // no AuthUser extractor. Router-level middleware still covers every
        // route in this chain by default, so it needs an explicit carve-out
        // here or it silently becomes not-actually-anonymous (confirmed live
        // with a real 401 before this fix -- work order item 5).
        || path == "/api/v1/rating/quote"
        // Prospect quoting is the other deliberately anonymous path (work
        // order item 2) -- capture at intent (a real premium exists to
        // save), not at entry. Rate-limited at the handler, not here.
        || path == "/api/v1/quotes/prospect"
        || dev_auth_path
}

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

    // Public routes — no auth required.
    if is_public_path(&path, state.config.dev_local_auth_enabled) {
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

#[cfg(test)]
mod tests {
    use super::is_public_path;

    #[test]
    fn rating_quote_is_public_regardless_of_dev_auth_setting() {
        // Work order item 5: rating must be reachable without credentials.
        // This must hold whether or not dev-local auth is enabled -- it's
        // not a dev-only carve-out, it's product-level anonymous pricing.
        assert!(is_public_path("/api/v1/rating/quote", false));
        assert!(is_public_path("/api/v1/rating/quote", true));
    }

    #[test]
    fn prospect_quote_is_public_regardless_of_dev_auth_setting() {
        // Work order item 2: same reasoning as rating/quote above.
        assert!(is_public_path("/api/v1/quotes/prospect", false));
        assert!(is_public_path("/api/v1/quotes/prospect", true));
    }

    #[test]
    fn health_checks_are_public() {
        assert!(is_public_path("/health", false));
        assert!(is_public_path("/api/v1/health", false));
        assert!(is_public_path("/api/v1/health/ready", false));
    }

    #[test]
    fn dev_auth_paths_are_public_only_when_dev_auth_is_enabled() {
        assert!(!is_public_path("/api/v1/auth/login", false));
        assert!(is_public_path("/api/v1/auth/login", true));
    }

    #[test]
    fn everything_else_requires_auth() {
        assert!(!is_public_path("/api/v1/quotes", false));
        assert!(!is_public_path("/api/v1/quotes", true));
        assert!(!is_public_path("/api/v1/quotes/:id/bind", false));
        assert!(!is_public_path("/api/v1/policies", false));
        // A path that merely starts similarly must not accidentally match.
        assert!(!is_public_path("/api/v1/rating/quote/extra", false));
        assert!(!is_public_path("/api/v1/rating-quote", false));
        assert!(!is_public_path("/api/v1/quotes/prospect/extra", false));
    }
}
