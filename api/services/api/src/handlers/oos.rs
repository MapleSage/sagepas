use axum::{Json, extract::State, http::StatusCode};
use oos_orchestrator::{OosEndorsementInput, OosError, OosResult};

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;
use domain::auth::PasRole;

const LIFECYCLE_MUTATION_ROLES: &[PasRole] =
    &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];

/// POST /api/v1/pas/oos-endorse
pub async fn oos_endorse(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(input): Json<OosEndorsementInput>,
) -> Result<Json<OosResult>, (StatusCode, String)> {
    require_roles(&user, LIFECYCLE_MUTATION_ROLES)?;
    let result = state
        .oos_orchestrator
        .apply(input)
        .await
        .map_err(map_oos_error)?;

    Ok(Json(result))
}

fn map_oos_error(error: OosError) -> (StatusCode, String) {
    match error {
        OosError::PolicyNotFound => (StatusCode::NOT_FOUND, error.to_string()),
        OosError::ConcurrentModification => (StatusCode::LOCKED, error.to_string()),
        OosError::NoBaseVersionFound
        | OosError::DatabaseError(_)
        | OosError::UnbalancedJournal(_) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}
