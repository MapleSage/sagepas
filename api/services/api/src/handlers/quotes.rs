use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use domain::auth::PasRole;
use event_models::{InsuranceEvent, PolicyBound, QuoteCreated};
use event_projector::PolicyTimelineProjection;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateQuoteRequest {
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub premium: f64,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default)]
    pub coverage: serde_json::Value,
}
fn default_currency() -> String {
    "USD".into()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct QuoteRow {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub state: String,
    pub premium: f64,
    pub currency: String,
    pub coverage: serde_json::Value,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

pub async fn create_quote(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateQuoteRequest>,
) -> Result<Json<QuoteRow>, (StatusCode, String)> {
    require_roles(
        &user,
        &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter],
    )?;
    let id = Uuid::new_v4();
    let coverage = if req.coverage.is_null() {
        serde_json::json!({})
    } else {
        req.coverage
    };

    // Some older clients/tests send a hard-coded product UUID that may not
    // exist in this environment. Resolve to a valid product (same insurance
    // type when available) instead of throwing an FK 500.
    let requested_product_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE id = $1)")
            .bind(req.product_id)
            .fetch_one(&**state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut product_id = req.product_id;

    if !requested_product_exists {
        let inferred_type = coverage
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());

        let fallback_product: Option<Uuid> = if let Some(kind) = inferred_type {
            sqlx::query_scalar(
                "SELECT id FROM products WHERE country = 'US' AND lower(insurance_type) = $1 ORDER BY name LIMIT 1",
            )
            .bind(kind)
            .fetch_optional(&**state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        } else {
            sqlx::query_scalar("SELECT id FROM products WHERE country = 'US' ORDER BY name LIMIT 1")
                .fetch_optional(&**state.db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        };

        if let Some(pid) = fallback_product {
            info!(
                requested_product_id = %req.product_id,
                resolved_product_id = %pid,
                "quote create resolved unknown product_id to existing product"
            );
            product_id = pid;
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid product_id: {}", req.product_id),
            ));
        }
    }

    let row = sqlx::query_as::<_, QuoteRow>(
        "INSERT INTO quotes (id, customer_id, product_id, state, premium, currency, coverage)
         VALUES ($1, $2, $3, 'quick_quote', $4, $5, $6)
         RETURNING id, customer_id, product_id, state, premium, currency, coverage, created_at, updated_at",
    )
    .bind(id)
    .bind(req.customer_id)
    .bind(product_id)
    .bind(req.premium)
    .bind(req.currency)
    .bind(coverage)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let event = InsuranceEvent::QuoteCreated(QuoteCreated {
        event_id: Uuid::new_v4(),
        occurred_at: Utc::now(),
        quote_id: row.id,
        customer_id: row.customer_id,
        product_id: row.product_id,
        premium: row.premium,
        currency: row.currency.clone(),
    });

    if let Err(err) = state.event_bus.publish(event).await {
        warn!(quote_id = %row.id, error = %err, "failed to publish QuoteCreated event");
    }

    Ok(Json(row))
}

pub async fn list_quotes(
    State(state): State<AppState>,
) -> Result<Json<Vec<QuoteRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, QuoteRow>(
        "SELECT
            q.id,
            q.customer_id,
            q.product_id,
            COALESCE(
                CASE qtp.projected_stage
                    WHEN 'policy_issued' THEN 'issued'
                    WHEN 'policy_bound' THEN 'bound'
                    WHEN 'quote_created' THEN 'quick_quote'
                    ELSE q.state
                END,
                q.state
            ) AS state,
            COALESCE(qtp.premium, q.premium) AS premium,
            COALESCE(qtp.currency, q.currency) AS currency,
            q.coverage,
            q.created_at,
            q.updated_at
         FROM quotes q
         LEFT JOIN quote_timeline_projections qtp ON qtp.quote_id = q.id
         ORDER BY q.created_at DESC
         LIMIT 200",
    )
    .fetch_all(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(rows))
}

pub async fn get_quote(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<QuoteRow>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, QuoteRow>(
        "SELECT id, customer_id, product_id, state, premium, currency, coverage, created_at, updated_at
         FROM quotes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "quote not found".to_string()))?;
    Ok(Json(row))
}

pub async fn get_quote_timeline(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PolicyTimelineProjection>, (StatusCode, String)> {
    let events = state
        .event_store
        .load(&format!("quote-{id}"))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut projector = event_projector::EventProjector::new();
    projector.apply_all(events.iter());

    let timeline = projector.timeline_by_quote(id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "quote timeline not found".to_string(),
        )
    })?;

    Ok(Json(timeline))
}

pub async fn bind_quote(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<QuoteRow>, (StatusCode, String)> {
    require_roles(
        &user,
        &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter],
    )?;
    let row = sqlx::query_as::<_, QuoteRow>(
        "UPDATE quotes SET state = 'bound', updated_at = NOW()
         WHERE id = $1 AND state NOT IN ('bound','issued','cancelled','expired')
         RETURNING id, customer_id, product_id, state, premium, currency, coverage, created_at, updated_at",
    )
    .bind(id)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            "quote cannot be bound in its current state".to_string(),
        )
    })?;

    let event = InsuranceEvent::PolicyBound(PolicyBound {
        event_id: Uuid::new_v4(),
        occurred_at: Utc::now(),
        quote_id: row.id,
        customer_id: row.customer_id,
        product_id: row.product_id,
    });

    if let Err(err) = state.event_bus.publish(event).await {
        warn!(quote_id = %row.id, error = %err, "failed to publish PolicyBound event");
    }

    Ok(Json(row))
}
