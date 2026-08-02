use async_trait::async_trait;
use domain::insurance::{Currency, PremiumCalculation, RateFactor};

use crate::underwriting::{evaluate_auto, AutoRiskProfile, UnderwritingDecision};
use crate::{
    ExecutionMode, ProviderIdentity, RatingDecision, RatingError, RatingProvider, RatingRequest,
    RatingResult,
};

/// Native PAS rating adapter. Pricing remains owned by the Rust pricing engine;
/// this provider validates its complete auditable calculation, then runs it
/// through deterministic underwriting before promoting it into a decision.
/// Underwriting can accept (with a risk loading), refer for manual review, or
/// decline outright — it is not a pass-through: see `underwriting.rs`.
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
            "AED" => Currency::Aed,
            other => {
                return Err(RatingError::MappingFailed(format!(
                    "unsupported pricing currency: {other}"
                )));
            }
        };
        let mut factors: Vec<RateFactor> = serde_json::from_value(
            pricing
                .get("factors")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|error| RatingError::MappingFailed(format!("pricing.factors: {error}")))?;

        // Underwriting is Auto-only for now (scoped deliberately — see the
        // rating crate's underwriting module doc comment). A quote is
        // treated as Auto when a vehicle_value is present; other lines are
        // promoted as-is pending underwriting coverage for those products.
        let vehicle_value = request.risk.get("vehicle_value").and_then(|v| v.as_f64());
        if vehicle_value.is_none() {
            return Ok(RatingResult {
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
            });
        }

        let profile = AutoRiskProfile {
            driver_age: request
                .risk
                .get("customer_age")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            prior_claims_5y: request
                .risk
                .get("prior_claims_count")
                .and_then(|v| v.as_i64()),
            coverage_amount: request.risk.get("coverage_amount").and_then(|v| v.as_f64()),
            vehicle_value,
        };
        let underwriting = evaluate_auto(&profile);

        match underwriting.decision {
            UnderwritingDecision::Declined => Ok(RatingResult {
                decision: RatingDecision::Declined,
                premium: None,
                referral_reason: Some(underwriting.reasons.join("; ")),
                carrier: request.carrier_id,
                product: request.product_id,
                provider: self.identity(),
            }),
            UnderwritingDecision::Referred => Ok(RatingResult {
                decision: RatingDecision::Referred,
                premium: None,
                referral_reason: Some(underwriting.reasons.join("; ")),
                carrier: request.carrier_id,
                product: request.product_id,
                provider: self.identity(),
            }),
            UnderwritingDecision::Quoted => {
                factors.extend(underwriting.factors);
                let underwritten_premium =
                    (final_premium * underwriting.risk_multiplier * 100.0).round() / 100.0;
                Ok(RatingResult {
                    decision: RatingDecision::Quoted,
                    premium: Some(PremiumCalculation {
                        base_rate,
                        factors,
                        final_premium: underwritten_premium,
                        currency,
                    }),
                    referral_reason: None,
                    carrier: request.carrier_id,
                    product: request.product_id,
                    provider: self.identity(),
                })
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(carrier_id: &str, pricing: serde_json::Value) -> RatingRequest {
        RatingRequest {
            carrier_id: carrier_id.to_string(),
            product_id: "auto-us".to_string(),
            state: "TX".to_string(),
            risk: serde_json::json!({ "pricing": pricing }),
        }
    }

    #[tokio::test]
    async fn promotes_native_pricing_without_recalculating_premium() {
        let provider = PasProvider::new();
        let result = provider
            .rate(request(
                "sagesure_pas",
                serde_json::json!({
                    "base_rate": 1_500.0,
                    "final_premium": 1_800.0,
                    "currency": "USD",
                    "factors": [{
                        "name": "vehicle_value",
                        "value": 1.2,
                        "description": "Vehicle value factor"
                    }]
                }),
            ))
            .await
            .expect("native pricing should produce a quote");

        assert!(matches!(result.decision, RatingDecision::Quoted));
        let premium = result.premium.expect("quoted result must contain premium");
        assert_eq!(premium.base_rate, 1_500.0);
        assert_eq!(premium.final_premium, 1_800.0);
        assert_eq!(result.provider.id, "pas:native:v1");
    }

    #[tokio::test]
    async fn rejects_missing_native_pricing() {
        let provider = PasProvider::new();
        let error = provider
            .rate(RatingRequest {
                carrier_id: "pas".to_string(),
                product_id: "auto-us".to_string(),
                state: "TX".to_string(),
                risk: serde_json::json!({}),
            })
            .await
            .expect_err("pricing evidence is required");

        assert!(matches!(error, RatingError::NotSupported { .. }));
    }

    #[tokio::test]
    async fn rejects_non_positive_premium() {
        let provider = PasProvider::new();
        let error = provider
            .rate(request(
                "pas",
                serde_json::json!({
                    "base_rate": 100.0,
                    "final_premium": 0.0,
                    "currency": "USD"
                }),
            ))
            .await
            .expect_err("zero premium must not be quoted");

        assert!(matches!(error, RatingError::MappingFailed(_)));
    }

    #[test]
    fn supports_only_native_pas_carrier_aliases() {
        let provider = PasProvider::new();
        assert!(provider.supports(&request("pas", serde_json::json!({}))));
        assert!(provider.supports(&request("SAGESURE_PAS", serde_json::json!({}))));
        assert!(!provider.supports(&request("unconfigured_carrier", serde_json::json!({}))));
    }

    fn auto_request(risk_extra: serde_json::Value, pricing: serde_json::Value) -> RatingRequest {
        let mut risk = risk_extra;
        risk["pricing"] = pricing;
        RatingRequest {
            carrier_id: "sagesure_pas".to_string(),
            product_id: "auto-us".to_string(),
            state: "TX".to_string(),
            risk,
        }
    }

    fn clean_pricing() -> serde_json::Value {
        serde_json::json!({
            "base_rate": 1_200.0,
            "final_premium": 1_200.0,
            "currency": "USD",
            "factors": []
        })
    }

    #[tokio::test]
    async fn auto_quote_with_clean_risk_is_quoted_with_underwriting_factors_attached() {
        let provider = PasProvider::new();
        let result = provider
            .rate(auto_request(
                serde_json::json!({
                    "customer_age": 35,
                    "coverage_amount": 20_000.0,
                    "vehicle_value": 25_000.0,
                    "prior_claims_count": 0
                }),
                clean_pricing(),
            ))
            .await
            .expect("clean auto risk should be quoted");

        assert!(matches!(result.decision, RatingDecision::Quoted));
        let premium = result.premium.expect("quoted result must contain premium");
        // no surcharge-triggering factors, multiplier is 1.0
        assert_eq!(premium.final_premium, 1_200.0);
        assert!(premium.factors.iter().any(|f| f.name == "driver_age"));
        assert!(premium
            .factors
            .iter()
            .any(|f| f.name == "prior_claims_5y"));
    }

    #[tokio::test]
    async fn auto_quote_with_three_prior_claims_is_declined_end_to_end() {
        let provider = PasProvider::new();
        let result = provider
            .rate(auto_request(
                serde_json::json!({
                    "customer_age": 35,
                    "coverage_amount": 20_000.0,
                    "vehicle_value": 25_000.0,
                    "prior_claims_count": 3
                }),
                clean_pricing(),
            ))
            .await
            .expect("declined is a successful RatingResult, not an error");

        assert!(matches!(result.decision, RatingDecision::Declined));
        assert!(result.premium.is_none(), "declined quotes must not carry a bindable premium");
        assert!(result
            .referral_reason
            .expect("decline must carry a reason")
            .contains("3 claims"));
    }

    #[tokio::test]
    async fn auto_quote_with_two_prior_claims_is_referred_end_to_end() {
        let provider = PasProvider::new();
        let result = provider
            .rate(auto_request(
                serde_json::json!({
                    "customer_age": 35,
                    "coverage_amount": 20_000.0,
                    "vehicle_value": 25_000.0,
                    "prior_claims_count": 2
                }),
                clean_pricing(),
            ))
            .await
            .expect("referred is a successful RatingResult, not an error");

        assert!(matches!(result.decision, RatingDecision::Referred));
        assert!(result.premium.is_none(), "referred quotes must not carry a bindable premium");
        assert!(result
            .referral_reason
            .expect("referral must carry a reason")
            .contains("2 claims"));
    }

    #[tokio::test]
    async fn auto_quote_applies_risk_multiplier_on_top_of_pricing_premium() {
        let provider = PasProvider::new();
        let result = provider
            .rate(auto_request(
                serde_json::json!({
                    "customer_age": 35,
                    "coverage_amount": 90_000.0,
                    "vehicle_value": 100_000.0, // high-value band, 1.35x
                    "prior_claims_count": 1 // 1.25x
                }),
                clean_pricing(), // final_premium 1200.0 before underwriting
            ))
            .await
            .expect("elevated but acceptable risk should still be quoted");

        assert!(matches!(result.decision, RatingDecision::Quoted));
        let premium = result.premium.expect("quoted result must contain premium");
        let expected: f64 = (1_200.0 * 1.25 * 1.35 * 100.0_f64).round() / 100.0;
        assert_eq!(premium.final_premium, expected);
        assert!(premium.final_premium > 1_200.0, "risk loadings must actually change the bound premium");
    }

    #[tokio::test]
    async fn non_auto_quote_without_vehicle_value_still_promotes_pricing_as_before() {
        let provider = PasProvider::new();
        let result = provider
            .rate(auto_request(
                serde_json::json!({}), // no vehicle_value => not treated as Auto
                clean_pricing(),
            ))
            .await
            .expect("non-auto lines pass through pending underwriting coverage");

        assert!(matches!(result.decision, RatingDecision::Quoted));
    }
}
