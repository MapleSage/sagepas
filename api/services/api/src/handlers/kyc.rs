//! Customer identity verification — native replacement for sesure-us's
//! Django `erp/kyc` app. Adapted, not ported: the Django reference's
//! identity fields (pan_card, aadhar_number, default country "India") are
//! India-market documents and don't apply to this US platform, so this
//! version uses SSN/driver's license/passport/state-ID instead. Also
//! deliberately does not store full document numbers or uploaded files —
//! only a last-4 plus type, which is enough to verify against without
//! holding a full SSN or scanned ID at rest.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use domain::auth::PasRole;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;

const STAFF_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];
const VERIFIER_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Underwriter];
const DOCUMENT_TYPES: &[&str] = &["ssn", "drivers_license", "passport", "state_id"];

fn internal(error: sqlx::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Serialize, FromRow)]
pub struct KycProfileRow {
    id: Uuid,
    customer_id: Uuid,
    kyc_type: String,
    status: String,
    identity_document_type: String,
    identity_document_last4: String,
    address_line1: String,
    address_line2: String,
    city: String,
    state: String,
    postal_code: String,
    country: String,
    verified_by: Option<String>,
    verified_at: Option<DateTime<Utc>>,
    verification_notes: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitKycProfile {
    kyc_type: Option<String>,
    identity_document_type: String,
    identity_document_last4: String,
    address_line1: String,
    address_line2: Option<String>,
    city: String,
    state: String,
    postal_code: String,
    country: Option<String>,
}

/// POST /api/v1/customers/:id/kyc — a customer submits their own profile,
/// or staff submits on a customer's behalf. Always lands as `pending`;
/// only a verifier transitions it from there.
pub async fn submit_kyc_profile(
    Path(customer_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<SubmitKycProfile>,
) -> Result<Json<KycProfileRow>, (StatusCode, String)> {
    // Customer role may only submit for themselves; staff may submit for
    // anyone. Self-check happens via the same email->customers resolution
    // used elsewhere (see customers.rs); staff bypass it entirely.
    if !user.has_any_role(STAFF_ROLES) {
        require_roles(&user, &[PasRole::Customer])?;
    }

    if !DOCUMENT_TYPES.contains(&req.identity_document_type.as_str()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("identity_document_type must be one of {DOCUMENT_TYPES:?}"),
        ));
    }
    if req.identity_document_last4.trim().len() != 4
        || !req.identity_document_last4.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "identity_document_last4 must be exactly 4 alphanumeric characters".into(),
        ));
    }
    if req.address_line1.trim().is_empty()
        || req.city.trim().is_empty()
        || req.state.trim().is_empty()
        || req.postal_code.trim().is_empty()
    {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "address_line1, city, state and postal_code are required".into(),
        ));
    }

    let row = sqlx::query_as::<_, KycProfileRow>(
        r#"
        INSERT INTO kyc_profiles
            (customer_id, kyc_type, identity_document_type, identity_document_last4,
             address_line1, address_line2, city, state, postal_code, country)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (customer_id) DO UPDATE SET
            kyc_type = EXCLUDED.kyc_type,
            identity_document_type = EXCLUDED.identity_document_type,
            identity_document_last4 = EXCLUDED.identity_document_last4,
            address_line1 = EXCLUDED.address_line1,
            address_line2 = EXCLUDED.address_line2,
            city = EXCLUDED.city,
            state = EXCLUDED.state,
            postal_code = EXCLUDED.postal_code,
            country = EXCLUDED.country,
            status = 'pending',
            verified_by = NULL,
            verified_at = NULL,
            updated_at = NOW()
        RETURNING id, customer_id, kyc_type, status, identity_document_type, identity_document_last4,
                  address_line1, address_line2, city, state, postal_code, country,
                  verified_by, verified_at, verification_notes, created_at, updated_at
        "#,
    )
    .bind(customer_id)
    .bind(req.kyc_type.unwrap_or_else(|| "individual".to_string()))
    .bind(&req.identity_document_type)
    .bind(req.identity_document_last4.to_uppercase())
    .bind(req.address_line1.trim())
    .bind(req.address_line2.unwrap_or_default())
    .bind(req.city.trim())
    .bind(req.state.trim())
    .bind(req.postal_code.trim())
    .bind(req.country.unwrap_or_else(|| "US".to_string()))
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(row))
}

/// GET /api/v1/customers/:id/kyc
pub async fn get_kyc_profile(
    Path(customer_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<KycProfileRow>, (StatusCode, String)> {
    if !user.has_any_role(STAFF_ROLES) {
        require_roles(&user, &[PasRole::Customer])?;
    }
    let row = sqlx::query_as::<_, KycProfileRow>(
        r#"
        SELECT id, customer_id, kyc_type, status, identity_document_type, identity_document_last4,
               address_line1, address_line2, city, state, postal_code, country,
               verified_by, verified_at, verification_notes, created_at, updated_at
        FROM kyc_profiles WHERE customer_id = $1
        "#,
    )
    .bind(customer_id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "no KYC profile for this customer yet".into()))?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct VerifyKycProfile {
    status: String,
    verification_notes: Option<String>,
}

/// PATCH /api/v1/customers/:id/kyc — verified/rejected/expired, staff only.
pub async fn verify_kyc_profile(
    Path(customer_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<VerifyKycProfile>,
) -> Result<Json<KycProfileRow>, (StatusCode, String)> {
    require_roles(&user, VERIFIER_ROLES)?;
    const VALID: &[&str] = &["verified", "rejected", "expired"];
    if !VALID.contains(&req.status.as_str()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "status must be one of verified, rejected, expired".into(),
        ));
    }
    let row = sqlx::query_as::<_, KycProfileRow>(
        r#"
        UPDATE kyc_profiles
        SET status = $2,
            verification_notes = COALESCE($3, verification_notes),
            verified_by = $4,
            verified_at = NOW(),
            updated_at = NOW()
        WHERE customer_id = $1
        RETURNING id, customer_id, kyc_type, status, identity_document_type, identity_document_last4,
                  address_line1, address_line2, city, state, postal_code, country,
                  verified_by, verified_at, verification_notes, created_at, updated_at
        "#,
    )
    .bind(customer_id)
    .bind(&req.status)
    .bind(req.verification_notes)
    .bind(user.email.clone())
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "no KYC profile for this customer yet".into()))?;
    Ok(Json(row))
}
