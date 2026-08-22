use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{NaiveDate, Utc};
use domain::auth::PasRole;
use premium_ledger::{BatchInput, ClaimReservePosition, JournalEntryInput, JournalLineInput};
use rust_decimal::Decimal;
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

/// Converts a validated-finite dollar amount to the fixed-precision Decimal
/// the ledger stores (NUMERIC(18,4)) via its formatted string, not a direct
/// float cast -- avoids binary-float rounding surprises landing in
/// accounting entries that must net to exactly zero.
fn money(amount: f64) -> Result<Decimal, (StatusCode, String)> {
    use std::str::FromStr;
    Decimal::from_str(&format!("{amount:.4}"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// One journal entry moving `delta` between the case reserve liability and
/// its offsetting loss expense -- shared by the initial notification posting
/// (delta = the opening reserve) and every re-estimation (delta = the
/// change). `delta` may be negative (a reserve decrease); the two lines
/// always net to zero regardless of sign.
fn case_reserve_batch(
    policy_id: Uuid,
    claim_id: Uuid,
    event_type: &str,
    effective_date: NaiveDate,
    delta: Decimal,
    currency: &str,
) -> BatchInput {
    BatchInput {
        policy_id,
        event_type: event_type.to_string(),
        entries: vec![JournalEntryInput {
            effective_date,
            lines: vec![
                JournalLineInput {
                    account_no: "5010".to_string(),
                    amount: delta,
                    currency: currency.to_string(),
                },
                JournalLineInput {
                    account_no: "2410".to_string(),
                    amount: -delta,
                    currency: currency.to_string(),
                },
            ],
        }],
        claim_id: Some(claim_id),
    }
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
    let reserve_amount = money(req.reserve_amount)?;
    let claim_number = format!(
        "CLM-{}",
        &Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );

    let mut tx = state.db.begin().await.map_err(internal)?;

    let row = sqlx::query_as::<_, ClaimRow>("INSERT INTO policy_claims (policy_id, claim_number, loss_date, loss_type, description, reserve_amount, currency) SELECT id, $2, $3, $4, $5, $6, currency FROM policies WHERE id = $1 RETURNING id, policy_id, claim_number, loss_date, loss_type, description, status, reserve_amount, currency::text AS currency, created_at, updated_at")
        .bind(id).bind(claim_number).bind(req.loss_date).bind(req.loss_type.trim()).bind(req.description.trim()).bind(req.reserve_amount)
        .fetch_optional(&mut *tx).await.map_err(internal)?.ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".into()))?;

    // Reserve posting on claim notification (work order item 4): only when
    // there's an actual opening reserve to record -- journal_lines forbids
    // a zero-amount line, and a zero reserve is genuinely nothing to post.
    if !reserve_amount.is_zero() {
        state
            .subledger
            .post_batch_in_transaction(
                &mut tx,
                case_reserve_batch(
                    row.policy_id,
                    row.id,
                    "claim_reserve_notification",
                    Utc::now().date_naive(),
                    reserve_amount,
                    &row.currency,
                ),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    tx.commit().await.map_err(internal)?;

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

#[derive(Debug, Deserialize)]
pub struct ReestimateReserve {
    new_reserve_amount: f64,
    /// Business date this re-estimate takes effect. Defaults to today;
    /// backdating it (e.g. correcting an earlier misestimate) is what makes
    /// the as-of query below restate history from that date forward.
    #[serde(default)]
    effective_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct ReserveReestimateResponse {
    claim: ClaimRow,
    previous_reserve_amount: f64,
    delta: f64,
    posted: bool,
}

/// PATCH /api/v1/policies/:id/claims/:claim_id/reserve
///
/// Reserve movement on re-estimation (work order item 4). Posts only the
/// delta between the current and new reserve, as its own batch distinct
/// from the notification batch -- each re-estimate is its own originating
/// event, not a correction folded into the first one.
pub async fn reestimate_claim_reserve(
    Path((id, claim_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<ReestimateReserve>,
) -> Result<Json<ReserveReestimateResponse>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    if !req.new_reserve_amount.is_finite() || req.new_reserve_amount < 0.0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "new_reserve_amount must be a non-negative number".into(),
        ));
    }
    let new_reserve = money(req.new_reserve_amount)?;
    let effective_date = req.effective_date.unwrap_or_else(|| Utc::now().date_naive());

    let mut tx = state.db.begin().await.map_err(internal)?;

    let current = sqlx::query_as::<_, ClaimRow>(
        "SELECT id, policy_id, claim_number, loss_date, loss_type, description, status, reserve_amount, currency::text AS currency, created_at, updated_at \
         FROM policy_claims WHERE id = $1 AND policy_id = $2 FOR UPDATE",
    )
    .bind(claim_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "claim not found".into()))?;

    let previous_reserve = money(current.reserve_amount)?;
    let delta = new_reserve - previous_reserve;

    let posted = if !delta.is_zero() {
        state
            .subledger
            .post_batch_in_transaction(
                &mut tx,
                case_reserve_batch(
                    current.policy_id,
                    claim_id,
                    "claim_reserve_reestimation",
                    effective_date,
                    delta,
                    &current.currency,
                ),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        true
    } else {
        false
    };

    let updated = sqlx::query_as::<_, ClaimRow>(
        "UPDATE policy_claims SET reserve_amount = $3, updated_at = NOW() WHERE id = $1 AND policy_id = $2 \
         RETURNING id, policy_id, claim_number, loss_date, loss_type, description, status, reserve_amount, currency::text AS currency, created_at, updated_at",
    )
    .bind(claim_id)
    .bind(id)
    .bind(req.new_reserve_amount)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;

    tx.commit().await.map_err(internal)?;

    if posted {
        notify(
            &state,
            id,
            "claim",
            "Claim reserve re-estimated",
            format!(
                "Claim {} case reserve re-estimated from {:.2} to {:.2} {} effective {}.",
                updated.claim_number,
                current.reserve_amount,
                updated.reserve_amount,
                updated.currency,
                effective_date
            ),
        )
        .await?;
    }

    Ok(Json(ReserveReestimateResponse {
        claim: updated,
        previous_reserve_amount: current.reserve_amount,
        delta: req.new_reserve_amount - current.reserve_amount,
        posted,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ReserveAsOfQuery {
    date: NaiveDate,
}

/// GET /api/v1/policies/:id/claims/:claim_id/reserve?date=YYYY-MM-DD
///
/// Bitemporal restatement query (work order item 4's demonstrable part):
/// the case reserve and losses-incurred position as they stood, business-
/// date-wise, as of `date` -- including any backdated re-estimation whose
/// effective_date falls on or before it, even if that correction was
/// recorded well after the fact.
pub async fn get_claim_reserve_as_of(
    Path((id, claim_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<ReserveAsOfQuery>,
    State(state): State<AppState>,
) -> Result<Json<ClaimReservePosition>, (StatusCode, String)> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM policy_claims WHERE id = $1 AND policy_id = $2)",
    )
    .bind(claim_id)
    .bind(id)
    .fetch_one(&**state.db)
    .await
    .map_err(internal)?;
    if !exists {
        return Err((StatusCode::NOT_FOUND, "claim not found".into()));
    }

    let position = state
        .subledger
        .get_claim_reserve_position(claim_id, query.date)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(position))
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
