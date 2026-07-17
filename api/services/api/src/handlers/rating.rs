use axum::{Json, http::StatusCode};
use rating::{PasProvider, RatingError, RatingProvider, RatingRequest};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RatingQuoteRequest {
    pub carrier_id: String,
    pub product_id: String,
    pub state: String,
    pub risk: serde_json::Value,
}

pub async fn quote(
    Json(req): Json<RatingQuoteRequest>,
) -> Result<Json<rating::RatingResult>, (StatusCode, String)> {
    let request = RatingRequest {
        carrier_id: req.carrier_id,
        product_id: req.product_id,
        state: req.state,
        risk: req.risk,
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
