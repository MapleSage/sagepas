//! Axum extractor and role-enforcement helper for the identity attached to
//! a request by `middleware::require_auth`. Handlers that need to know who
//! is calling, or need to gate an operation to specific PAS roles, pull
//! `AuthUser` as a normal extractor argument.

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::{StatusCode, request::Parts};
use domain::auth::{AuthenticatedUser, PasRole};
use uuid::Uuid;

use crate::state::AppState;

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

/// Resolves the calling `Customer`-role identity to their `customers.id`
/// (work order Phase 8), creating the row just-in-time on first sign-in if
/// none exists yet — see `handlers::customers::resolve_or_create_customer_id`
/// for the email-matching/JIT-creation logic. This is what makes
/// `PasRole::Customer` load-bearing for the first time: routes under `/my/*`
/// (Phase 4) pull `CustomerContext` instead of `AuthUser` so every query is
/// scoped to `WHERE customer_id = $1`, not just gated on a role flag with no
/// real record behind it.
#[derive(Debug, Clone, Copy)]
pub struct CustomerContext(pub Uuid);

#[async_trait]
impl FromRequestParts<AppState> for CustomerContext {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        require_roles(&user, &[PasRole::Customer])?;
        let email = user.email.as_deref().ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "token has no email claim; cannot resolve a customer identity".to_string(),
            )
        })?;
        let id = crate::handlers::customers::resolve_or_create_customer_id(
            state,
            email,
            user.name.as_deref(),
        )
        .await?;
        Ok(CustomerContext(id))
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
