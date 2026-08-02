//! Deterministic Auto underwriting decisioning.
//!
//! Every factor here is a fixed table keyed on a real, verifiable input —
//! no model call, no free-text scoring. Each factor independently resolves
//! to accept (with a rate multiplier), refer, or decline; the aggregate
//! decision is decline > refer > quoted so any single hard-stop factor
//! wins. This is deliberately conservative: it is meant to prove decline
//! and refer are real, reachable code paths, not to be a finished
//! actuarial model.

use domain::insurance::RateFactor;

/// The subset of a risk profile Auto underwriting evaluates.
/// All fields are `Option` because upstream data can be missing — a
/// missing field that underwriting depends on is itself a reason to
/// refer or decline, never a reason to silently skip the check.
#[derive(Debug, Clone, Default)]
pub struct AutoRiskProfile {
    pub driver_age: Option<u32>,
    /// Number of claims filed by this customer in the last 5 years,
    /// looked up from the real `claims` table by the caller — never
    /// client-supplied.
    pub prior_claims_5y: Option<i64>,
    pub coverage_amount: Option<f64>,
    pub vehicle_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactorOutcome {
    Accept,
    Refer,
    Decline,
}

#[derive(Debug, Clone)]
pub struct FactorResult {
    pub factor: RateFactor,
    pub outcome: FactorOutcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnderwritingDecision {
    Quoted,
    Referred,
    Declined,
}

#[derive(Debug, Clone)]
pub struct UnderwritingResult {
    pub decision: UnderwritingDecision,
    /// Combined multiplier of every ACCEPTED factor's loading. Only
    /// meaningful when `decision == Quoted`.
    pub risk_multiplier: f64,
    pub factors: Vec<RateFactor>,
    pub reasons: Vec<String>,
}

/// Evaluate driver age eligibility and age-band loading.
fn evaluate_age(age: Option<u32>, coverage_amount: Option<f64>) -> FactorResult {
    match age {
        None => FactorResult {
            factor: RateFactor {
                name: "driver_age".into(),
                value: 1.0,
                description: Some("driver age not provided".into()),
            },
            outcome: FactorOutcome::Refer,
            reason: Some("driver age is required to rate an auto risk".into()),
        },
        Some(age) if age < 18 => FactorResult {
            factor: RateFactor {
                name: "driver_age".into(),
                value: 1.0,
                description: Some(format!("age {age} below minimum insurable age")),
            },
            outcome: FactorOutcome::Decline,
            reason: Some(format!(
                "driver age {age} is below the minimum insurable age of 18"
            )),
        },
        Some(age) if age <= 24 => {
            let high_coverage = coverage_amount.unwrap_or(0.0) > 50_000.0;
            if high_coverage {
                FactorResult {
                    factor: RateFactor {
                        name: "driver_age".into(),
                        value: 1.5,
                        description: Some(format!("age {age}, high coverage requested")),
                    },
                    outcome: FactorOutcome::Refer,
                    reason: Some(format!(
                        "young driver (age {age}) requesting coverage above $50,000 requires manual review"
                    )),
                }
            } else {
                FactorResult {
                    factor: RateFactor {
                        name: "driver_age".into(),
                        value: 1.5,
                        description: Some(format!("young driver surcharge, age {age}")),
                    },
                    outcome: FactorOutcome::Accept,
                    reason: None,
                }
            }
        }
        Some(age) if age <= 70 => FactorResult {
            factor: RateFactor {
                name: "driver_age".into(),
                value: 1.0,
                description: Some(format!("standard age band, age {age}")),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(age) if age <= 80 => FactorResult {
            factor: RateFactor {
                name: "driver_age".into(),
                value: 1.2,
                description: Some(format!("senior surcharge, age {age}")),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(age) => FactorResult {
            factor: RateFactor {
                name: "driver_age".into(),
                value: 1.2,
                description: Some(format!("age {age} above 80")),
            },
            outcome: FactorOutcome::Refer,
            reason: Some(format!(
                "driver age {age} is above 80 and requires senior-risk specialist review"
            )),
        },
    }
}

/// Evaluate prior-claims history (last 5 years, from the real claims table).
fn evaluate_prior_claims(count: Option<i64>) -> FactorResult {
    match count {
        None => FactorResult {
            factor: RateFactor {
                name: "prior_claims_5y".into(),
                value: 1.0,
                description: Some("claims history unavailable".into()),
            },
            outcome: FactorOutcome::Refer,
            reason: Some("prior claims history could not be verified".into()),
        },
        Some(0) => FactorResult {
            factor: RateFactor {
                name: "prior_claims_5y".into(),
                value: 1.0,
                description: Some("no claims in last 5 years".into()),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(1) => FactorResult {
            factor: RateFactor {
                name: "prior_claims_5y".into(),
                value: 1.25,
                description: Some("1 claim in last 5 years".into()),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(2) => FactorResult {
            factor: RateFactor {
                name: "prior_claims_5y".into(),
                value: 1.6,
                description: Some("2 claims in last 5 years".into()),
            },
            outcome: FactorOutcome::Refer,
            reason: Some("2 claims in the last 5 years requires manual underwriter review".into()),
        },
        Some(n) => FactorResult {
            factor: RateFactor {
                name: "prior_claims_5y".into(),
                value: 1.0,
                description: Some(format!("{n} claims in last 5 years")),
            },
            outcome: FactorOutcome::Decline,
            reason: Some(format!(
                "{n} claims in the last 5 years exceeds the maximum of 2 for straight-through issuance"
            )),
        },
    }
}

/// Evaluate coverage-to-vehicle-value ratio (insurable-interest / over-insurance check).
fn evaluate_coverage_ratio(coverage_amount: Option<f64>, vehicle_value: Option<f64>) -> FactorResult {
    let coverage = coverage_amount.unwrap_or(0.0);
    match vehicle_value {
        None | Some(0.0) => FactorResult {
            factor: RateFactor {
                name: "coverage_to_value_ratio".into(),
                value: 1.0,
                description: Some("vehicle value not provided".into()),
            },
            outcome: FactorOutcome::Refer,
            reason: Some(
                "vehicle value is required to validate coverage against insurable interest".into(),
            ),
        },
        Some(value) => {
            let ratio = coverage / value;
            if ratio <= 1.2 {
                FactorResult {
                    factor: RateFactor {
                        name: "coverage_to_value_ratio".into(),
                        value: 1.0,
                        description: Some(format!("ratio {ratio:.2}")),
                    },
                    outcome: FactorOutcome::Accept,
                    reason: None,
                }
            } else if ratio <= 1.5 {
                FactorResult {
                    factor: RateFactor {
                        name: "coverage_to_value_ratio".into(),
                        value: 1.0,
                        description: Some(format!("ratio {ratio:.2}")),
                    },
                    outcome: FactorOutcome::Refer,
                    reason: Some(format!(
                        "requested coverage is {ratio:.2}x vehicle value; possible over-insurance requires review"
                    )),
                }
            } else {
                FactorResult {
                    factor: RateFactor {
                        name: "coverage_to_value_ratio".into(),
                        value: 1.0,
                        description: Some(format!("ratio {ratio:.2}")),
                    },
                    outcome: FactorOutcome::Decline,
                    reason: Some(format!(
                        "requested coverage is {ratio:.2}x vehicle value, exceeding the 1.5x insurable-interest limit"
                    )),
                }
            }
        }
    }
}

/// Evaluate vehicle value band as a proxy for vehicle risk class.
fn evaluate_vehicle_value_band(vehicle_value: Option<f64>) -> FactorResult {
    match vehicle_value {
        None | Some(0.0) => FactorResult {
            factor: RateFactor {
                name: "vehicle_value_band".into(),
                value: 1.0,
                description: Some("vehicle value not provided".into()),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(value) if value <= 75_000.0 => FactorResult {
            factor: RateFactor {
                name: "vehicle_value_band".into(),
                value: 1.0,
                description: Some("standard risk class".into()),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(value) if value <= 150_000.0 => FactorResult {
            factor: RateFactor {
                name: "vehicle_value_band".into(),
                value: 1.35,
                description: Some(format!("high-value vehicle, ${value:.0}")),
            },
            outcome: FactorOutcome::Accept,
            reason: None,
        },
        Some(value) => FactorResult {
            factor: RateFactor {
                name: "vehicle_value_band".into(),
                value: 1.35,
                description: Some(format!("vehicle value ${value:.0} above $150,000")),
            },
            outcome: FactorOutcome::Refer,
            reason: Some(format!(
                "vehicle value ${value:.0} exceeds $150,000 and requires specialist review"
            )),
        },
    }
}

/// Evaluate the full Auto risk profile against all factors and aggregate
/// to a single decision. Decline beats refer beats quoted: any hard stop
/// wins regardless of how many other factors would have accepted.
pub fn evaluate_auto(profile: &AutoRiskProfile) -> UnderwritingResult {
    let results = vec![
        evaluate_age(profile.driver_age, profile.coverage_amount),
        evaluate_prior_claims(profile.prior_claims_5y),
        evaluate_coverage_ratio(profile.coverage_amount, profile.vehicle_value),
        evaluate_vehicle_value_band(profile.vehicle_value),
    ];

    let declined: Vec<&FactorResult> = results
        .iter()
        .filter(|r| r.outcome == FactorOutcome::Decline)
        .collect();
    let referred: Vec<&FactorResult> = results
        .iter()
        .filter(|r| r.outcome == FactorOutcome::Refer)
        .collect();

    let factors: Vec<RateFactor> = results.iter().map(|r| r.factor.clone()).collect();

    if !declined.is_empty() {
        return UnderwritingResult {
            decision: UnderwritingDecision::Declined,
            risk_multiplier: 1.0,
            factors,
            reasons: declined.iter().filter_map(|r| r.reason.clone()).collect(),
        };
    }

    if !referred.is_empty() {
        return UnderwritingResult {
            decision: UnderwritingDecision::Referred,
            risk_multiplier: 1.0,
            factors,
            reasons: referred.iter().filter_map(|r| r.reason.clone()).collect(),
        };
    }

    let risk_multiplier: f64 = results.iter().map(|r| r.factor.value).product();

    UnderwritingResult {
        decision: UnderwritingDecision::Quoted,
        risk_multiplier,
        factors,
        reasons: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_profile() -> AutoRiskProfile {
        AutoRiskProfile {
            driver_age: Some(35),
            prior_claims_5y: Some(0),
            coverage_amount: Some(20_000.0),
            vehicle_value: Some(25_000.0),
        }
    }

    #[test]
    fn clean_risk_is_quoted() {
        let result = evaluate_auto(&base_profile());
        assert_eq!(result.decision, UnderwritingDecision::Quoted);
        assert_eq!(result.risk_multiplier, 1.0);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn three_prior_claims_is_declined() {
        let mut profile = base_profile();
        profile.prior_claims_5y = Some(3);
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Declined);
        assert!(!result.reasons.is_empty());
        assert!(result.reasons[0].contains("3 claims"));
    }

    #[test]
    fn over_insurance_is_declined() {
        let mut profile = base_profile();
        profile.coverage_amount = Some(40_000.0);
        profile.vehicle_value = Some(20_000.0); // ratio 2.0 > 1.5
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Declined);
        assert!(result.reasons[0].contains("insurable-interest"));
    }

    #[test]
    fn underage_driver_is_declined() {
        let mut profile = base_profile();
        profile.driver_age = Some(16);
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Declined);
        assert!(result.reasons[0].contains("minimum insurable age"));
    }

    #[test]
    fn two_prior_claims_is_referred_not_declined() {
        let mut profile = base_profile();
        profile.prior_claims_5y = Some(2);
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Referred);
        assert!(result.reasons[0].contains("2 claims"));
    }

    #[test]
    fn young_driver_high_coverage_is_referred() {
        let mut profile = base_profile();
        profile.driver_age = Some(20);
        profile.coverage_amount = Some(60_000.0);
        profile.vehicle_value = Some(55_000.0); // keeps ratio factor from also firing
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Referred);
        assert!(result.reasons.iter().any(|r| r.contains("young driver")));
    }

    #[test]
    fn high_value_vehicle_is_referred() {
        let mut profile = base_profile();
        profile.vehicle_value = Some(200_000.0);
        profile.coverage_amount = Some(180_000.0); // ratio 0.9, stays under the ratio decline/refer bar
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Referred);
        assert!(result.reasons[0].contains("specialist review"));
    }

    #[test]
    fn decline_wins_over_refer_when_both_present() {
        let mut profile = base_profile();
        profile.prior_claims_5y = Some(2); // would refer
        profile.driver_age = Some(16); // declines
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Declined);
    }

    #[test]
    fn missing_age_is_referred_not_silently_accepted() {
        let mut profile = base_profile();
        profile.driver_age = None;
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Referred);
    }

    #[test]
    fn accepted_risk_loadings_compound_into_multiplier() {
        let mut profile = base_profile();
        profile.prior_claims_5y = Some(1); // 1.25x
        profile.vehicle_value = Some(100_000.0); // 1.35x, still <=150k
        profile.coverage_amount = Some(90_000.0); // ratio 0.9, accept
        let result = evaluate_auto(&profile);
        assert_eq!(result.decision, UnderwritingDecision::Quoted);
        assert!((result.risk_multiplier - (1.25 * 1.35)).abs() < 1e-9);
    }
}
