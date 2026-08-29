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
        // NOTE (work order §12.2, HELD 2026-08-25): connect.rs's own doc
        // comment claims GIA chat is anonymous-capable, but this global
        // middleware 401s any unauthenticated call before that handler
        // logic ever runs -- so it's currently dead code. Opening
        // /api/v1/connect/chat here is explicitly NOT approved yet: an
        // anonymous LLM-backed endpoint needs per-IP rate limiting,
        // per-session token ceilings, and a spend cap demonstrated failing
        // closed first (cost objection, not security -- this operation has
        // a real prior $38k/4B-token runaway inference incident). Do not
        // add this path here until all three are built and the cap has
        // been driven past its limit and shown to refuse.
        //
        // WhatsApp webhook (Phase 7): Meta calls this directly with no
        // bearer token available. It is verified via the X-Hub-Signature-256
        // HMAC header instead (see handlers::notify::verify_whatsapp_signature)
        // -- not left open, just authenticated a different way. The sibling
        // send/send-flow endpoints are deliberately NOT public: those require
        // an authenticated staff caller.
        || path == "/api/v1/notify/whatsapp/webhook"
        || dev_auth_path
}

/// Routes sesure-us's `65ee45ec-...` staff audience is accepted on (work
/// order Phase 9, delegated backend reads) -- see `AppConfig::
/// azure_entra_delegate_audience`'s doc comment for why this list exists
/// separately from a blanket audience add. Extend this list one tier at a
/// time as each read is actually moved, not ahead of it.
const STAFF_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];

fn is_delegate_readable_path(path: &str) -> bool {
    matches!(path, "/api/v1/products")
}

/// Validates `token` against the delegate (sesure-us staff audience)
/// validator, requiring a real STAFF_ROLES claim on top of a valid
/// signature/tenant/audience. Both checks failing look identical to the
/// caller (`Ok(None)`) -- deliberately not distinguishing "right audience,
/// wrong role" from "wrong audience" in the response, same posture as any
/// other rejected validator attempt in `try_validate`.
async fn try_delegate(
    state: &AppState,
    token: &str,
) -> Result<Option<AuthenticatedUser>, (StatusCode, String)> {
    let Some(validator) = &state.entra_delegate_validator else {
        return Ok(None);
    };
    let Ok(user) = validator.validate(token).await else {
        return Ok(None);
    };
    if !user.has_any_role(STAFF_ROLES) {
        return Ok(None);
    }
    Ok(Some(user))
}

/// Validates a Bearer token if one is present. Returns `Ok(None)` -- not an
/// error -- when there's simply no Authorization header, so a lenient
/// (public) path can proceed anonymous instead of 401ing. A *malformed or
/// invalid* token is still a hard error even on a lenient path; "no token"
/// and "bad token" are different things and only the first is tolerated
/// here. Work order §12.1.
async fn try_validate(
    headers: &axum::http::HeaderMap,
    state: &AppState,
    path: &str,
) -> Result<Option<AuthenticatedUser>, (StatusCode, String)> {
    let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };

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
            if state.entra_staff_validator.is_none() && state.entra_consumer_validator.is_none() {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Entra authentication is not configured on this server".to_string(),
                ));
            }

            // Try staff then consumer -- each independently rejects on a
            // mismatched `tid`/`aud`, so trying both configured validators
            // is cheap and safe. A token that matches neither surfaces the
            // staff validator's error (arbitrary but stable choice; the
            // real signal for the caller is "no configured tenant accepted
            // this token", not which one specifically said no first).
            let staff_result = match &state.entra_staff_validator {
                Some(v) => Some(v.validate(token).await),
                None => None,
            };
            let delegate_user = if staff_result.is_none() || staff_result.as_ref().is_some_and(|r| r.is_err()) {
                if is_delegate_readable_path(path) {
                    try_delegate(state, token).await?
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(Ok(user)) = staff_result {
                user
            } else if let Some(user) = delegate_user {
                user
            } else {
                let consumer_result = match &state.entra_consumer_validator {
                    Some(v) => Some(v.validate(token).await),
                    None => None,
                };
                match consumer_result {
                    Some(Ok(user)) => user,
                    _ => {
                        let err = staff_result
                            .and_then(|r| r.err())
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "no configured Entra tenant accepted this token".to_string());
                        return Err((StatusCode::UNAUTHORIZED, format!("invalid Entra token: {err}")));
                    }
                }
            }
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

    Ok(Some(user))
}

/// Axum middleware that validates the caller's identity on every request.
/// On a public path (see `is_public_path`): no token proceeds anonymous
/// (unchanged behavior), a *valid* token attaches the caller's identity
/// anyway (new, work order §12.1 -- e.g. so a signed-in staff member gets
/// role-scoped context on an otherwise-anonymous-capable path), and a
/// malformed/invalid token still hard-fails either way. Every other path
/// requires a valid token, as before.
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

    if is_public_path(&path, state.config.dev_local_auth_enabled) {
        if let Some(user) = try_validate(req.headers(), &state, &path).await? {
            req.extensions_mut().insert(user);
        }
        return Ok(next.run(req).await);
    }

    let user = try_validate(req.headers(), &state, &path)
        .await?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "missing Authorization header".to_string()))?;

    // Attach the validated identity to request extensions for handlers to read.
    req.extensions_mut().insert(user);

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::{is_delegate_readable_path, is_public_path};

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
    fn whatsapp_webhook_is_public_but_send_is_not() {
        // Phase 7: the webhook is authenticated via HMAC signature instead
        // of a bearer token, so it must be public here -- but the send/
        // send-flow endpoints must stay auth-required.
        assert!(is_public_path("/api/v1/notify/whatsapp/webhook", false));
        assert!(!is_public_path("/api/v1/notify/whatsapp/send", false));
        assert!(!is_public_path("/api/v1/notify/whatsapp/send-flow", false));
    }

    #[test]
    fn dashboard_stats_requires_auth() {
        assert!(!is_public_path("/api/v1/dashboard/stats", false));
    }

    #[test]
    fn only_the_explicit_tier_is_delegate_readable() {
        // Phase 9: products is tier 1. Nothing else is allowlisted yet --
        // extend this test alongside is_delegate_readable_path as each new
        // tier actually moves, not ahead of it.
        assert!(is_delegate_readable_path("/api/v1/products"));
        assert!(!is_delegate_readable_path("/api/v1/dashboard/stats"));
        assert!(!is_delegate_readable_path("/api/v1/customers"));
        assert!(!is_delegate_readable_path("/api/v1/policies/:id"));
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
