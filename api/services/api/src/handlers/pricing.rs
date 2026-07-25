use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use domain::insurance::InsuranceType;
use pricing::PremiumRequest;

/// POST /api/v1/pricing/estimate
///
/// Returns a premium estimate for the given parameters.
/// Does NOT persist anything — purely a computation endpoint.
#[derive(Debug, Deserialize)]
pub struct EstimateRequest {
    pub insurance_type: InsuranceType,
    pub country: String,
    pub state_code: Option<String>,
    pub coverage_amount: f64,
    pub customer_age: Option<u32>,
    pub vehicle_value: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct EstimateResponse {
    pub base_rate: f64,
    pub factors: Vec<RateFactorDto>,
    pub final_premium: f64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub struct RateFactorDto {
    pub name: String,
    pub value: f64,
    pub description: Option<String>,
}

pub async fn estimate(
    State(state): State<AppState>,
    Json(req): Json<EstimateRequest>,
) -> Result<Json<EstimateResponse>, (StatusCode, String)> {
    let premium_req = PremiumRequest {
        insurance_type: req.insurance_type,
        country: req.country.clone(),
        state_code: req.state_code,
        coverage_amount: req.coverage_amount,
        customer_age: req.customer_age,
        vehicle_value: req.vehicle_value,
    };

    let calc = state
        .pricing
        .estimate(premium_req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let currency = match calc.currency {
        domain::insurance::Currency::Inr => "INR",
        domain::insurance::Currency::Usd => "USD",
        domain::insurance::Currency::Aed => "AED",
    };

    Ok(Json(EstimateResponse {
        base_rate: calc.base_rate,
        factors: calc
            .factors
            .into_iter()
            .map(|f| RateFactorDto {
                name: f.name,
                value: f.value,
                description: f.description,
            })
            .collect(),
        final_premium: calc.final_premium,
        currency: currency.to_string(),
    }))
}
