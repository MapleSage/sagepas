//! Claim fraud-risk scoring — native replacement for sesure-us's Django
//! `erp/fraud` app (schema existed, zero wiring, unauthenticated proxy).
//! Red-flag weighting follows that reference (FraudRisk.calculate_risk_score);
//! the flags themselves are recomputed here from data this schema actually
//! has, not carried over as-is — sagepas has no sum-insured/coverage-limit
//! field, so "over-claim" is a reserve-vs-premium heuristic, not a policy-
//! limit breach. This is a red flag for a human reviewer, not a verdict.

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

fn internal(error: sqlx::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Serialize, FromRow)]
pub struct FraudRiskRow {
    id: Uuid,
    claim_id: Uuid,
    policy_id: Uuid,
    risk_score: i32,
    risk_level: String,
    status: String,
    is_duplicate_claim: bool,
    is_over_claim: bool,
    high_claim_frequency: bool,
    description: String,
    investigation_notes: String,
    assigned_to: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Weighted the same as the Django reference (duplicate 30, over-claim 25,
/// high-frequency 15 out of its original 5-flag/100-point scale) minus the
/// two flags ("staged claim", "unusual pattern") that reference left as
/// manually-set booleans with no computation behind them -- not carried
/// over since nothing here can actually compute them from data alone.
fn score_and_level(is_duplicate: bool, is_over_claim: bool, high_frequency: bool) -> (i32, &'static str) {
    let mut score = 0;
    if is_duplicate {
        score += 30;
    }
    if is_over_claim {
        score += 25;
    }
    if high_frequency {
        score += 15;
    }
    let level = if score < 30 {
        "low"
    } else if score < 50 {
        "medium"
    } else if score < 80 {
        "high"
    } else {
        "critical"
    };
    (score, level)
}

/// POST /api/v1/claims/:id/fraud-risk — compute (or recompute) risk for a
/// claim from live data: same policy + same loss_type within 30 days
/// (duplicate), reserve far exceeding premium (over-claim), 3+ claims on
/// the policy in the last 12 months (high frequency).
pub async fn compute_fraud_risk(
    Path(claim_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<FraudRiskRow>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;

    #[derive(FromRow)]
    struct ClaimContext {
        policy_id: Uuid,
        loss_type: String,
        loss_date: chrono::NaiveDate,
        reserve_amount: f64,
        premium: f64,
    }
    let ctx = sqlx::query_as::<_, ClaimContext>(
        r#"
        SELECT c.policy_id, c.loss_type, c.loss_date, c.reserve_amount, p.premium
        FROM policy_claims c JOIN policies p ON p.id = c.policy_id
        WHERE c.id = $1
        "#,
    )
    .bind(claim_id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "claim not found".into()))?;

    let is_duplicate_claim = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM policy_claims
            WHERE policy_id = $1 AND loss_type = $2 AND id <> $3
              AND loss_date BETWEEN $4::date - INTERVAL '30 days' AND $4::date + INTERVAL '30 days'
        )
        "#,
    )
    .bind(ctx.policy_id)
    .bind(&ctx.loss_type)
    .bind(claim_id)
    .bind(ctx.loss_date)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;

    // Heuristic, not a limits check: this schema has no sum-insured field.
    let is_over_claim = ctx.premium > 0.0 && ctx.reserve_amount > ctx.premium * 10.0;

    let claim_count_12mo = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM policy_claims WHERE policy_id = $1 AND created_at > NOW() - INTERVAL '12 months'"#,
    )
    .bind(ctx.policy_id)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;
    let high_claim_frequency = claim_count_12mo >= 3;

    let (risk_score, risk_level) =
        score_and_level(is_duplicate_claim, is_over_claim, high_claim_frequency);

    let row = sqlx::query_as::<_, FraudRiskRow>(
        r#"
        INSERT INTO claim_fraud_risk
            (claim_id, policy_id, risk_score, risk_level, is_duplicate_claim, is_over_claim, high_claim_frequency)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (claim_id) DO UPDATE SET
            risk_score = EXCLUDED.risk_score,
            risk_level = EXCLUDED.risk_level,
            is_duplicate_claim = EXCLUDED.is_duplicate_claim,
            is_over_claim = EXCLUDED.is_over_claim,
            high_claim_frequency = EXCLUDED.high_claim_frequency,
            updated_at = NOW()
        RETURNING id, claim_id, policy_id, risk_score, risk_level, status,
                  is_duplicate_claim, is_over_claim, high_claim_frequency,
                  description, investigation_notes, assigned_to, created_at, updated_at
        "#,
    )
    .bind(claim_id)
    .bind(ctx.policy_id)
    .bind(risk_score)
    .bind(risk_level)
    .bind(is_duplicate_claim)
    .bind(is_over_claim)
    .bind(high_claim_frequency)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(row))
}

/// GET /api/v1/claims/:id/fraud-risk
pub async fn get_fraud_risk(
    Path(claim_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<FraudRiskRow>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    let row = sqlx::query_as::<_, FraudRiskRow>(
        r#"
        SELECT id, claim_id, policy_id, risk_score, risk_level, status,
               is_duplicate_claim, is_over_claim, high_claim_frequency,
               description, investigation_notes, assigned_to, created_at, updated_at
        FROM claim_fraud_risk WHERE claim_id = $1
        "#,
    )
    .bind(claim_id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "no fraud-risk assessment for this claim yet".into()))?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateFraudStatus {
    status: String,
    investigation_notes: Option<String>,
}

/// PATCH /api/v1/claims/:id/fraud-risk — reviewer moves the case through
/// the investigation workflow. Score/flags are not editable here; only
/// `compute_fraud_risk` recomputes them, so the score always traces back
/// to data, never to a manual override.
pub async fn update_fraud_status(
    Path(claim_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdateFraudStatus>,
) -> Result<Json<FraudRiskRow>, (StatusCode, String)> {
    require_roles(&user, &[PasRole::Admin, PasRole::Underwriter])?;
    const VALID: &[&str] = &["flagged", "under_investigation", "confirmed", "cleared", "closed"];
    if !VALID.contains(&req.status.as_str()) {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid status".into()));
    }
    let row = sqlx::query_as::<_, FraudRiskRow>(
        r#"
        UPDATE claim_fraud_risk
        SET status = $2,
            investigation_notes = COALESCE($3, investigation_notes),
            assigned_to = $4,
            updated_at = NOW()
        WHERE claim_id = $1
        RETURNING id, claim_id, policy_id, risk_score, risk_level, status,
                  is_duplicate_claim, is_over_claim, high_claim_frequency,
                  description, investigation_notes, assigned_to, created_at, updated_at
        "#,
    )
    .bind(claim_id)
    .bind(&req.status)
    .bind(req.investigation_notes)
    .bind(user.email.clone())
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "no fraud-risk assessment for this claim yet".into()))?;
    Ok(Json(row))
}
