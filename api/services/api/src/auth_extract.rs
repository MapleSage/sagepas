//! Axum extractor and role-enforcement helper for the identity attached to
//! a request by `middleware::require_auth`. Handlers that need to know who
//! is calling, or need to gate an operation to specific PAS roles, pull
//! `AuthUser` as a normal extractor argument.

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::{StatusCode, request::Parts};
use domain::auth::{AuthenticatedUser, PasRole};

/// Newtype wrapper around `domain::auth::AuthenticatedUser` so this crate
/// can implement the foreign `FromRequestParts` trait for it (Rust's
/// orphan rule forbids implementing a foreign trait for a foreign type).
#[derive(Debug, Clone)]
pub struct AuthUser(pub AuthenticatedUser);

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .map(AuthUser)
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    "authentication required".to_string(),
                )
            })
    }
}

/// Rejects the request unless the authenticated user holds at least one of
/// the allowed PAS roles. Roles come only from validated token claims —
/// there is no path here that trusts client-supplied role input.
pub fn require_roles(
    user: &AuthenticatedUser,
    allowed: &[PasRole],
) -> Result<(), (StatusCode, String)> {
    if user.has_any_role(allowed) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!(
                "role not permitted for this operation (have: {:?}, need one of: {:?})",
                user.pas_roles, allowed
            ),
        ))
    }
}
