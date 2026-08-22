//! Work order item 2: an unauthenticated quote path for prospects who have
//! no PAS customer yet. `create_quote` requires a staff role and an
//! existing `customer_id` -- there was no path at all for "anonymous person
//! rates, then gives their email to save it." This is that path.
//!
//! Deliberately does not gate `/api/v1/rating/quote` and does not put
//! registration in front of pricing -- identity is captured only once a
//! real premium exists to save, and only as name/email/phone (no password,
//! no portal account).

use axum::{Json, extract::State, http::HeaderMap, http::StatusCode};
use rating::{PasProvider, RatingDecision, RatingError, RatingProvider, RatingRequest, RatingResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::handlers::customers::{
    CreateCustomerRequest, CustomerRow, customer_write_error, insert_customer_in_tx,
    validate_customer,
};
use crate::handlers::quotes::{QuoteRow, resolve_product_id};
use crate::hubspot_bridge;
use crate::state::AppState;

pub const RATE_LIMIT_MAX_REQUESTS: usize = 10;
pub const RATE_LIMIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProspectQuoteRequest {
    pub carrier_id: String,
    pub product_id: Uuid,
    pub state: String,
    #[serde(default)]
    pub risk: serde_json::Value,
    pub name: String,
    pub email: String,
    pub phone: String,
}

#[derive(Debug, Serialize)]
pub struct ProspectQuoteResponse {
    pub rating: RatingResult,
    /// Only present when `rating.decision == Quoted` -- a Referred or
    /// Declined outcome has no premium to persist, so nothing is written.
    pub quote: Option<QuoteRow>,
    pub customer_id: Option<Uuid>,
}

/// `Currency` has no Display impl and its serde form is lowercase
/// (`snake_case`), which doesn't match this codebase's ISO-uppercase
/// convention elsewhere (`quotes.rs`'s own `default_currency() -> "USD"`).
fn currency_code(currency: &domain::insurance::Currency) -> &'static str {
    use domain::insurance::Currency;
    match currency {
        Currency::Usd => "USD",
        Currency::Inr => "INR",
        Currency::Aed => "AED",
    }
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn map_rating_error(err: RatingError) -> (StatusCode, String) {
    let status = match err {
        RatingError::NotSupported { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        RatingError::MappingFailed(_) => StatusCode::BAD_REQUEST,
        RatingError::ExecutionFailed(_) | RatingError::Timeout(_) => StatusCode::BAD_GATEWAY,
    };
    let body =
        serde_json::to_string(&err).unwrap_or_else(|_| "{\"error\":\"rating_failed\"}".to_string());
    (status, body)
}

/// POST /api/v1/quotes/prospect -- unauthenticated, rate-limited.
pub async fn prospect_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProspectQuoteRequest>,
) -> Result<Json<ProspectQuoteResponse>, (StatusCode, String)> {
    if !state.prospect_rate_limiter.check(&client_ip(&headers)) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "too many quote requests from this address, try again shortly".to_string(),
        ));
    }

    let customer_req = validate_customer(CreateCustomerRequest {
        name: req.name,
        email: req.email,
        phone: req.phone,
        country: "US".to_string(),
        currency: "USD".to_string(),
        national_id: None,
        national_id_type: None,
        address: None,
    })?;

    // Same enrichment rating::quote does for an authenticated quote, minus
    // the prior-claims lookup -- a prospect has no customer_id yet, so
    // there's nothing to look up; the underwriting factor already treats
    // "unknown" as a refer, not a silent accept.
    let mut risk = req.risk;
    let mut insurance_type = sqlx::query_scalar::<_, String>(
        "SELECT insurance_type FROM products WHERE id = $1",
    )
    .bind(req.product_id)
    .fetch_one(&**state.db)
    .await
    .ok();
    if let Some(kind) = &insurance_type {
        risk["insurance_type"] = serde_json::json!(kind);
    } else {
        // Requested product_id wasn't real (resolve_product_id will fall
        // back below); fall back here too, from whatever the caller
        // supplied directly, so coverage-type inference for that fallback
        // has something real to go on instead of an empty product row.
        insurance_type = risk
            .get("insurance_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    let rating_request = RatingRequest {
        carrier_id: req.carrier_id,
        product_id: req.product_id.to_string(),
        state: req.state,
        risk,
    };

    let provider = PasProvider::new();
    if !provider.supports(&rating_request) {
        return Err(map_rating_error(RatingError::NotSupported {
            provider: provider.identity().id,
            reason: "standalone SagePAS accepts only the native PAS rating provider".to_string(),
        }));
    }
    let rating_result = provider.rate(rating_request).await.map_err(map_rating_error)?;

    if !matches!(rating_result.decision, RatingDecision::Quoted) {
        return Ok(Json(ProspectQuoteResponse {
            rating: rating_result,
            quote: None,
            customer_id: None,
        }));
    }
    let premium = rating_result.premium.clone().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "rating returned Quoted with no premium".to_string(),
        )
    })?;

    let coverage = serde_json::json!({ "type": insurance_type });
    let product_id = resolve_product_id(&state, req.product_id, &coverage).await?;

    let mut tx = state.db.begin().await.map_err(customer_write_error)?;

    // Same email-serialization pattern as create_customer, so a prospect
    // double-submitting (or two tabs) can't race past each other into two
    // customer rows for the same email.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&customer_req.email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(customer_write_error)?;

    let existing_customer: Option<CustomerRow> = sqlx::query_as(
        r#"
        SELECT id, user_id, name, email, phone, country, currency,
               national_id, national_id_type, address, created_at
        FROM customers WHERE LOWER(email) = $1
        "#,
    )
    .bind(&customer_req.email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(customer_write_error)?;

    // Either branch resolves + upserts the HubSpot link inside this same
    // transaction: the existing-customer branch does it explicitly here,
    // the new-customer branch gets it for free from insert_customer_in_tx
    // (work order item 3) -- one path, not two that can drift.
    let customer_id = match existing_customer {
        Some(row) => {
            hubspot_bridge::ensure_link(
                &mut tx,
                &state,
                row.id,
                &customer_req.email,
                &customer_req.name,
                &customer_req.phone,
            )
            .await?;
            row.id
        }
        None => insert_customer_in_tx(&mut tx, &state, customer_req).await?.id,
    };

    let quote = sqlx::query_as::<_, QuoteRow>(
        r#"
        INSERT INTO quotes (id, customer_id, product_id, state, premium, currency, coverage)
        VALUES ($1, $2, $3, 'quick_quote', $4, $5, $6)
        RETURNING id, customer_id, product_id, state, premium, currency, coverage, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(customer_id)
    .bind(product_id)
    .bind(premium.final_premium)
    .bind(currency_code(&premium.currency))
    .bind(coverage)
    .fetch_one(&mut *tx)
    .await
    .map_err(customer_write_error)?;

    tx.commit().await.map_err(customer_write_error)?;

    Ok(Json(ProspectQuoteResponse {
        rating: rating_result,
        quote: Some(quote),
        customer_id: Some(customer_id),
    }))
}
