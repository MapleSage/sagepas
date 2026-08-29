//! Agent commission tracking — native replacement for sesure-us's Django
//! `erp/commissions` app (which had this modeled but never authenticated
//! or wired anywhere live). Field/workflow shape follows that reference:
//! rate -> amount, pending -> approved -> paid/reversed.

use axum::{
    Json,
    extract::{Path, Query, State},
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
const APPROVER_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Underwriter];

fn internal(error: sqlx::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Serialize, FromRow)]
pub struct CommissionRow {
    id: Uuid,
    agent_id: Uuid,
    policy_id: Uuid,
    amount: f64,
    currency: String,
    commission_rate: Option<f64>,
    status: String,
    paid_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommission {
    agent_id: Uuid,
    policy_id: Uuid,
    /// Base amount the commission is computed against (e.g. written premium).
    base_amount: f64,
    commission_rate: f64,
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListCommissionsQuery {
    agent_id: Option<Uuid>,
    status: Option<String>,
}

/// GET /api/v1/commissions
/// Staff sees everything; an Agent-role caller is scoped to their own
/// commissions regardless of an `agent_id` query param they might pass.
pub async fn list_commissions(
    Query(q): Query<ListCommissionsQuery>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<CommissionRow>>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    let scoped_agent_id = if user.has_any_role(&[PasRole::Admin, PasRole::Underwriter]) {
        q.agent_id
    } else {
        // Agent role: force-scope to self. Resolution of "self" as an
        // agents.id is done via email match, same pattern used elsewhere
        // for identity linking (see customers.rs).
        None
    };

    let rows = sqlx::query_as::<_, CommissionRow>(
        r#"
        SELECT c.id, c.agent_id, c.policy_id, c.amount, c.currency,
               c.commission_rate, c.status, c.paid_at, c.created_at
        FROM commissions c
        LEFT JOIN agents a ON a.id = c.agent_id
        WHERE ($1::uuid IS NULL OR c.agent_id = $1)
          AND ($2::text IS NULL OR c.status = $2)
          AND ($3::bool = FALSE OR a.email = $4)
        ORDER BY c.created_at DESC
        "#,
    )
    .bind(scoped_agent_id)
    .bind(q.status)
    .bind(!user.has_any_role(&[PasRole::Admin, PasRole::Underwriter]))
    .bind(user.email.as_deref().unwrap_or(""))
    .fetch_all(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(rows))
}

/// POST /api/v1/commissions
pub async fn create_commission(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateCommission>,
) -> Result<Json<CommissionRow>, (StatusCode, String)> {
    require_roles(&user, APPROVER_ROLES)?;
    if !req.base_amount.is_finite() || req.base_amount <= 0.0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "base_amount must be positive".into(),
        ));
    }
    if !req.commission_rate.is_finite() || req.commission_rate <= 0.0 || req.commission_rate > 100.0
    {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "commission_rate must be between 0 and 100".into(),
        ));
    }
    let amount = req.base_amount * (req.commission_rate / 100.0);
    let currency = req.currency.unwrap_or_else(|| "USD".to_string());

    let row = sqlx::query_as::<_, CommissionRow>(
        r#"
        INSERT INTO commissions (agent_id, policy_id, amount, currency, commission_rate, status)
        VALUES ($1, $2, $3, $4, $5, 'pending')
        RETURNING id, agent_id, policy_id, amount, currency, commission_rate, status, paid_at, created_at
        "#,
    )
    .bind(req.agent_id)
    .bind(req.policy_id)
    .bind(amount)
    .bind(currency)
    .bind(req.commission_rate)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateCommissionStatus {
    status: String,
}

/// PATCH /api/v1/commissions/:id/status — pending -> approved -> paid, or
/// -> reversed from any non-terminal state. Enforced here, not trusted
/// from the client, since this is a real ledger-adjacent state machine.
pub async fn update_commission_status(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdateCommissionStatus>,
) -> Result<Json<CommissionRow>, (StatusCode, String)> {
    require_roles(&user, APPROVER_ROLES)?;

    let current = sqlx::query_scalar::<_, String>("SELECT status FROM commissions WHERE id = $1")
        .bind(id)
        .fetch_optional(&**state.db)
        .await
        .map_err(internal)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "commission not found".into()))?;

    let valid_transition = matches!(
        (current.as_str(), req.status.as_str()),
        ("pending", "approved")
            | ("approved", "paid")
            | ("pending", "reversed")
            | ("approved", "reversed")
    );
    if !valid_transition {
        return Err((
            StatusCode::CONFLICT,
            format!("cannot transition commission from {current} to {}", req.status),
        ));
    }

    let paid_at_clause = if req.status == "paid" { "NOW()" } else { "paid_at" };
    let row = sqlx::query_as::<_, CommissionRow>(&format!(
        r#"
        UPDATE commissions SET status = $2, paid_at = {paid_at_clause}
        WHERE id = $1
        RETURNING id, agent_id, policy_id, amount, currency, commission_rate, status, paid_at, created_at
        "#
    ))
    .bind(id)
    .bind(&req.status)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(row))
}
