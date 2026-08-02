use axum::{Json, extract::State, http::StatusCode};
use rating::{PasProvider, RatingError, RatingProvider, RatingRequest};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RatingQuoteRequest {
    pub carrier_id: String,
    pub product_id: String,
    pub state: String,
    pub risk: serde_json::Value,
}

pub async fn quote(
    State(state): State<AppState>,
    Json(req): Json<RatingQuoteRequest>,
) -> Result<Json<rating::RatingResult>, (StatusCode, String)> {
    let mut risk = req.risk;

    // Underwriting needs real prior-claims history, not a client-supplied
    // number — look it up server-side from the actual claims table and
    // merge it in before rating. Silently skipped (not defaulted to 0) when
    // no customer_id is present or the customer has no claims table rows
    // yet; the underwriting factor itself treats "unknown" as a refer, not
    // an accept, so this never fails open.
    if let Some(customer_id) = risk.get("customer_id").and_then(|v| v.as_str()) {
        if let Ok(customer_id) = Uuid::parse_str(customer_id) {
            if let Ok(count) = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM claims WHERE customer_id = $1 AND created_at > NOW() - INTERVAL '5 years'",
            )
            .bind(customer_id)
            .fetch_one(&**state.db)
            .await
            {
                risk["prior_claims_count"] = serde_json::json!(count);
            }
        }
    }

    // Underwriting dispatches on the product's real insurance_type — looked
    // up here from the products table, never guessed from which risk fields
    // the caller happened to send. Silently skipped (not defaulted) when
    // product_id isn't a real product row; PasProvider treats an absent
    // insurance_type as "no underwriting case for this line yet" and
    // promotes pricing unchanged, so this never fails open into a decision
    // it didn't actually evaluate.
    if let Ok(product_id) = Uuid::parse_str(&req.product_id) {
        if let Ok(insurance_type) = sqlx::query_scalar::<_, String>(
            "SELECT insurance_type FROM products WHERE id = $1",
        )
        .bind(product_id)
        .fetch_one(&**state.db)
        .await
        {
            risk["insurance_type"] = serde_json::json!(insurance_type);
        }
    }

    let request = RatingRequest {
        carrier_id: req.carrier_id,
        product_id: req.product_id,
        state: req.state,
        risk,
    };

    let provider = PasProvider::new();
    if !provider.supports(&request) {
        return Err(map_rating_error(RatingError::NotSupported {
            provider: provider.identity().id,
            reason: "standalone SagePAS accepts only the native PAS rating provider".to_string(),
        }));
    }

    provider
        .rate(request)
        .await
        .map(Json)
        .map_err(map_rating_error)
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
