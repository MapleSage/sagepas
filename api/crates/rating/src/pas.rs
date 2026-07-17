use async_trait::async_trait;
use domain::insurance::{Currency, PremiumCalculation, RateFactor};

use crate::{
    ExecutionMode, ProviderIdentity, RatingDecision, RatingError, RatingProvider, RatingRequest,
    RatingResult,
};

/// Native PAS rating adapter. Pricing remains owned by the Rust pricing engine;
/// this provider validates and promotes its complete auditable calculation into
/// the canonical rating decision rather than inventing a second premium.
pub struct PasProvider;

impl PasProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RatingProvider for PasProvider {
    async fn rate(&self, request: RatingRequest) -> Result<RatingResult, RatingError> {
        let pricing = request
            .risk
            .get("pricing")
            .ok_or_else(|| RatingError::NotSupported {
                provider: self.identity().id,
                reason: "native pricing calculation is required before PAS rating".to_string(),
            })?;

        let base_rate = pricing
            .get("base_rate")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| {
                RatingError::MappingFailed("pricing.base_rate is required".to_string())
            })?;
        let final_premium = pricing
            .get("final_premium")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| {
                RatingError::MappingFailed("pricing.final_premium is required".to_string())
            })?;
        if !base_rate.is_finite() || !final_premium.is_finite() || final_premium <= 0.0 {
            return Err(RatingError::MappingFailed(
                "native pricing values must be finite and final_premium must be positive"
                    .to_string(),
            ));
        }

        let currency = match pricing
            .get("currency")
            .and_then(|value| value.as_str())
            .unwrap_or("USD")
            .to_ascii_uppercase()
            .as_str()
        {
            "INR" => Currency::Inr,
            "USD" => Currency::Usd,
            other => {
                return Err(RatingError::MappingFailed(format!(
                    "unsupported pricing currency: {other}"
                )));
            }
        };
        let factors: Vec<RateFactor> = serde_json::from_value(
            pricing
                .get("factors")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|error| RatingError::MappingFailed(format!("pricing.factors: {error}")))?;

        Ok(RatingResult {
            decision: RatingDecision::Quoted,
            premium: Some(PremiumCalculation {
                base_rate,
                factors,
                final_premium,
                currency,
            }),
            referral_reason: None,
            carrier: request.carrier_id,
            product: request.product_id,
            provider: self.identity(),
        })
    }

    fn supports(&self, request: &RatingRequest) -> bool {
        request.carrier_id.eq_ignore_ascii_case("pas")
            || request.carrier_id.eq_ignore_ascii_case("sagesure_pas")
    }

    fn identity(&self) -> ProviderIdentity {
        ProviderIdentity {
            id: "pas:native:v1".to_string(),
            version: "1.0.0".to_string(),
            execution_mode: ExecutionMode::Pas,
        }
    }
}
