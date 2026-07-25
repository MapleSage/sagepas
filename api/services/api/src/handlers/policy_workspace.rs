use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{NaiveDate, Utc};
use domain::auth::PasRole;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;

const STAFF_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];

async fn ensure_policy(state: &AppState, id: Uuid) -> Result<(), (StatusCode, String)> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM policies WHERE id = $1)")
            .bind(id)
            .fetch_one(&**state.db)
            .await
            .map_err(internal)?;
    if exists {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, "policy not found".into()))
    }
}

fn internal(error: sqlx::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

async fn notify(
    state: &AppState,
    policy_id: Uuid,
    category: &str,
    subject: &str,
    message: String,
) -> Result<(), (StatusCode, String)> {
    sqlx::query("INSERT INTO policy_notifications (policy_id, category, subject, message) VALUES ($1, $2, $3, $4)")
        .bind(policy_id).bind(category).bind(subject).bind(message)
        .execute(&**state.db).await.map_err(internal)?;
    Ok(())
}

#[derive(Debug, Serialize, FromRow)]
pub struct PaymentRow {
    id: Uuid,
    policy_id: Uuid,
    amount: f64,
    currency: String,
    payment_method: String,
    status: String,
    reference: Option<String>,
    received_at: chrono::DateTime<Utc>,
    created_at: chrono::DateTime<Utc>,
}
#[derive(Debug, Deserialize)]
pub struct CreatePayment {
    amount: f64,
    payment_method: String,
    reference: Option<String>,
}

pub async fn list_payments(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<PaymentRow>>, (StatusCode, String)> {
    ensure_policy(&state, id).await?;
    let rows = sqlx::query_as::<_, PaymentRow>("SELECT id, policy_id, amount, currency::text AS currency, payment_method, status, reference, received_at, created_at FROM policy_payments WHERE policy_id = $1 ORDER BY created_at DESC")
        .bind(id).fetch_all(&**state.db).await.map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create_payment(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreatePayment>,
) -> Result<Json<PaymentRow>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    if !req.amount.is_finite() || req.amount <= 0.0 || req.payment_method.trim().is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "positive amount and payment method are required".into(),
        ));
    }
    let row = sqlx::query_as::<_, PaymentRow>("INSERT INTO policy_payments (policy_id, amount, currency, payment_method, reference) SELECT id, $2, currency, $3, $4 FROM policies WHERE id = $1 RETURNING id, policy_id, amount, currency::text AS currency, payment_method, status, reference, received_at, created_at")
        .bind(id).bind(req.amount).bind(req.payment_method.trim()).bind(req.reference).fetch_optional(&**state.db).await.map_err(internal)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".into()))?;
    notify(
        &state,
        id,
        "payment",
        "Payment received",
        format!(
            "Payment of {:.2} {} was recorded.",
            row.amount, row.currency
        ),
    )
    .await?;
    Ok(Json(row))
}

#[derive(Debug, Serialize, FromRow)]
pub struct ClaimRow {
    id: Uuid,
    policy_id: Uuid,
    claim_number: String,
    loss_date: NaiveDate,
    loss_type: String,
    description: String,
    status: String,
    reserve_amount: f64,
    currency: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}
#[derive(Debug, Deserialize)]
pub struct CreateClaim {
    loss_date: NaiveDate,
    loss_type: String,
    description: String,
    #[serde(default)]
    reserve_amount: f64,
}

pub async fn list_claims(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ClaimRow>>, (StatusCode, String)> {
    ensure_policy(&state, id).await?;
    let rows = sqlx::query_as::<_, ClaimRow>("SELECT id, policy_id, claim_number, loss_date, loss_type, description, status, reserve_amount, currency::text AS currency, created_at, updated_at FROM policy_claims WHERE policy_id = $1 ORDER BY created_at DESC")
        .bind(id).fetch_all(&**state.db).await.map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create_claim(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateClaim>,
) -> Result<Json<ClaimRow>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    if req.loss_type.trim().is_empty()
        || req.description.trim().is_empty()
        || !req.reserve_amount.is_finite()
        || req.reserve_amount < 0.0
    {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "loss type, description, and a non-negative reserve are required".into(),
        ));
    }
    let claim_number = format!(
        "CLM-{}",
        &Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );
    let row = sqlx::query_as::<_, ClaimRow>("INSERT INTO policy_claims (policy_id, claim_number, loss_date, loss_type, description, reserve_amount, currency) SELECT id, $2, $3, $4, $5, $6, currency FROM policies WHERE id = $1 RETURNING id, policy_id, claim_number, loss_date, loss_type, description, status, reserve_amount, currency::text AS currency, created_at, updated_at")
        .bind(id).bind(claim_number).bind(req.loss_date).bind(req.loss_type.trim()).bind(req.description.trim()).bind(req.reserve_amount)
        .fetch_optional(&**state.db).await.map_err(internal)?.ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".into()))?;
    notify(
        &state,
        id,
        "claim",
        "Claim reported",
        format!(
            "Claim {} was reported for loss date {}.",
            row.claim_number, row.loss_date
        ),
    )
    .await?;
    Ok(Json(row))
}

#[derive(Debug, Serialize, FromRow)]
pub struct NotificationRow {
    id: Uuid,
    policy_id: Uuid,
    category: String,
    subject: String,
    message: String,
    channel: String,
    status: String,
    created_at: chrono::DateTime<Utc>,
}
pub async fn list_notifications(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<NotificationRow>>, (StatusCode, String)> {
    ensure_policy(&state, id).await?;
    let rows = sqlx::query_as::<_, NotificationRow>("SELECT id, policy_id, category, subject, message, channel, status, created_at FROM policy_notifications WHERE policy_id = $1 ORDER BY created_at DESC")
        .bind(id).fetch_all(&**state.db).await.map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, FromRow)]
pub struct TransactionRow {
    transaction_id: Uuid,
    transaction_type: String,
    status: String,
    amount: f64,
    currency: String,
    effective_at: chrono::DateTime<Utc>,
    description: String,
}
pub async fn list_transactions(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TransactionRow>>, (StatusCode, String)> {
    ensure_policy(&state, id).await?;
    let rows = sqlx::query_as::<_, TransactionRow>(
        r#"
        SELECT version_id AS transaction_id, source AS transaction_type, state AS status,
               premium_cents::double precision / 100.0 AS amount, currency::text AS currency,
               sys_start AS effective_at, 'Policy lifecycle: ' || source AS description
        FROM policy_versions WHERE policy_id = $1
        UNION ALL
        SELECT id, 'payment', status, amount, currency::text, received_at,
               'Payment via ' || payment_method || COALESCE(' (' || reference || ')', '')
        FROM policy_payments WHERE policy_id = $1
        UNION ALL
        SELECT id, 'claim', status, reserve_amount, currency::text, created_at,
               claim_number || ': ' || loss_type
        FROM policy_claims WHERE policy_id = $1
        ORDER BY effective_at DESC
    "#,
    )
    .bind(id)
    .fetch_all(&**state.db)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, FromRow)]
pub struct RenewalRow {
    id: Uuid,
    policy_id: Uuid,
    renewal_quote_id: Uuid,
    effective_date: NaiveDate,
    status: String,
    created_at: chrono::DateTime<Utc>,
}
pub async fn list_renewals(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<RenewalRow>>, (StatusCode, String)> {
    ensure_policy(&state, id).await?;
    let rows = sqlx::query_as::<_, RenewalRow>("SELECT id, policy_id, renewal_quote_id, effective_date, status, created_at FROM policy_renewals WHERE policy_id = $1 ORDER BY created_at DESC")
        .bind(id).fetch_all(&**state.db).await.map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create_renewal(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<RenewalRow>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    ensure_policy(&state, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let quote_id = Uuid::new_v4();
    let inserted = sqlx::query("INSERT INTO quotes (id, customer_id, product_id, state, premium, currency, coverage) SELECT $2, customer_id, product_id, 'quoted', premium, currency, coverage FROM policies WHERE id = $1")
        .bind(id).bind(quote_id).execute(&mut *tx).await.map_err(internal)?;
    if inserted.rows_affected() != 1 {
        return Err((StatusCode::NOT_FOUND, "policy not found".into()));
    }
    let row = sqlx::query_as::<_, RenewalRow>("INSERT INTO policy_renewals (policy_id, renewal_quote_id, effective_date) SELECT id, $2, end_date::date FROM policies WHERE id = $1 RETURNING id, policy_id, renewal_quote_id, effective_date, status, created_at")
        .bind(id).bind(quote_id).fetch_one(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    notify(
        &state,
        id,
        "renewal",
        "Renewal quote created",
        format!(
            "Renewal quote {} is ready for review.",
            row.renewal_quote_id
        ),
    )
    .await?;
    Ok(Json(row))
}
