use domain::insurance::{Currency, InsuranceType, PremiumCalculation, RateFactor};
use infra::search::SearchClient;
use tracing::{debug, warn};

/// Input parameters for a premium estimate request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PremiumRequest {
    pub insurance_type: InsuranceType,
    /// ISO 3166-1 alpha-2 country code (currently US, IN, or AE).
    pub country: String,
    /// US state code (e.g. "CT", "CA") for state-level rating.
    pub state_code: Option<String>,
    /// Coverage / sum-insured amount in the product currency.
    pub coverage_amount: f64,
    /// Customer age used for life / health rating.
    pub customer_age: Option<u32>,
    /// Vehicle market value (auto only).
    pub vehicle_value: Option<f64>,
}

/// Pricing engine that derives rate factors from the Azure AI Search
/// knowledge base and computes a final premium.
pub struct PricingEngine {
    search: SearchClient,
}

impl PricingEngine {
    pub fn new(search: SearchClient) -> Self {
        Self { search }
    }

    /// Estimate the annual premium for the given request.
    ///
    /// The engine:
    /// 1. Selects the appropriate KB index for the insurance line.
    /// 2. Performs a text search using the request parameters as a query.
    /// 3. Extracts numeric rate factors from the search results.
    /// 4. Returns `final_premium = base_rate * product(all factors)`.
    ///
    /// Sensible static defaults are used when the search index is
    /// unavailable or returns no results.
    pub async fn estimate(&self, req: PremiumRequest) -> anyhow::Result<PremiumCalculation> {
        let index = index_for_type(&req.insurance_type);
        let query = build_query(&req);
        let currency = currency_for_country(&req.country);

        // Derive base rate from coverage amount and static per-mille rates.
        let (base_rate, default_factors) = default_rates(&req);

        debug!(
            index,
            query,
            base_rate,
            insurance_type = ?req.insurance_type,
            "pricing engine starting search"
        );

        // Attempt to augment with search-derived factors.
        let search_factors = match self.search.text_search(index, &query, 5).await {
            Ok(results) => extract_factors(results),
            Err(err) => {
                warn!(%err, index, "search unavailable; using static defaults");
                vec![]
            }
        };

        let mut factors = default_factors;
        factors.extend(search_factors);

        // Compute final premium as base_rate multiplied by all factors.
        let multiplier: f64 = factors.iter().map(|f| f.value).product();
        let final_premium = (base_rate * multiplier * 100.0).round() / 100.0;

        Ok(PremiumCalculation {
            base_rate,
            factors,
            final_premium,
            currency,
        })
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Map insurance type to the Azure AI Search index name.
fn index_for_type(t: &InsuranceType) -> &'static str {
    match t {
        InsuranceType::Auto => "insurance-auto-kb",
        InsuranceType::Life => "insurance-life-kb",
        InsuranceType::Health => "insurance-health-kb",
        InsuranceType::Property => "insurance-property-kb",
        InsuranceType::Marine => "insurance-marine-kb",
    }
}

/// Build a free-text query string from the request fields.
fn build_query(req: &PremiumRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{:?} insurance", req.insurance_type).to_lowercase());
    parts.push(format!("country {}", req.country));
    if let Some(state) = &req.state_code {
        parts.push(format!("state {state}"));
    }
    if let Some(age) = req.customer_age {
        parts.push(format!("age {age}"));
    }
    if let Some(val) = req.vehicle_value {
        parts.push(format!("vehicle value {val}"));
    }
    parts.push(format!("coverage {}", req.coverage_amount));
    parts.join(" ")
}

fn currency_for_country(country: &str) -> Currency {
    if country.eq_ignore_ascii_case("IN") {
        Currency::Inr
    } else if country.eq_ignore_ascii_case("AE") {
        Currency::Aed
    } else {
        Currency::Usd
    }
}

/// Return sensible static (base_rate, factor list) defaults per insurance type.
fn default_rates(req: &PremiumRequest) -> (f64, Vec<RateFactor>) {
    let mut factors: Vec<RateFactor> = Vec::new();

    let base_rate = match req.insurance_type {
        InsuranceType::Auto => {
            // $1 200 / yr base for US auto
            let base = if req.country.eq_ignore_ascii_case("IN") {
                800.0_f64 // ₹800 base
            } else if req.country.eq_ignore_ascii_case("AE") {
                1500.0_f64 // AED 1,500 base
            } else {
                1200.0_f64
            };
            // Vehicle value surcharge: 1 % of vehicle value on top of base
            if let Some(val) = req.vehicle_value {
                factors.push(RateFactor {
                    name: "vehicle_value_load".to_string(),
                    value: 1.0 + (val * 0.0001).min(0.5), // cap at +50 %
                    description: Some(format!("load based on vehicle value ${val:.0}")),
                });
            }
            base
        }
        InsuranceType::Life => {
            // $500 / yr base; age band factors
            let base = if req.country.eq_ignore_ascii_case("IN") {
                400.0_f64
            } else if req.country.eq_ignore_ascii_case("AE") {
                650.0_f64
            } else {
                500.0_f64
            };
            if let Some(age) = req.customer_age {
                let age_factor = match age {
                    0..=30 => 1.0,
                    31..=45 => 1.3,
                    46..=60 => 1.8,
                    _ => 2.5,
                };
                factors.push(RateFactor {
                    name: "age_band".to_string(),
                    value: age_factor,
                    description: Some(format!("mortality load for age {age}")),
                });
            }
            base
        }
        InsuranceType::Health => {
            let base = if req.country.eq_ignore_ascii_case("IN") {
                600.0
            } else if req.country.eq_ignore_ascii_case("AE") {
                1800.0
            } else {
                1500.0
            };
            if let Some(age) = req.customer_age {
                let age_factor = if age < 35 {
                    1.0
                } else if age < 55 {
                    1.4
                } else {
                    2.0
                };
                factors.push(RateFactor {
                    name: "age_band".to_string(),
                    value: age_factor,
                    description: Some(format!("health risk load for age {age}")),
                });
            }
            base
        }
        InsuranceType::Property => {
            // 0.5 % of coverage amount
            req.coverage_amount * 0.005
        }
        InsuranceType::Marine => {
            // 0.3 % of coverage amount
            req.coverage_amount * 0.003
        }
    };

    (base_rate, factors)
}

/// Attempt to parse numeric rate factors from search result snippets.
///
/// The function looks for patterns like "rate: 1.2" or "factor 0.95" in the
/// content text.  This is best-effort; if nothing useful is found the list is
/// empty and the caller falls back to the static defaults.
fn extract_factors(results: Vec<infra::search::SearchResult>) -> Vec<RateFactor> {
    let mut out = Vec::new();

    for (i, result) in results.iter().enumerate() {
        // Very simple heuristic: look for a decimal number near the word "rate" or "factor".
        let content = result.content.to_lowercase();
        let Some(pos) = content.find("rate").or_else(|| content.find("factor")) else {
            continue;
        };
        let snippet = &content[pos..];
        // Find first decimal number in the snippet.
        let value = snippet
            .split_whitespace()
            .skip(1)
            .take(3)
            .find_map(|tok| tok.trim_matches([',', ';', ':']).parse::<f64>().ok());

        if let Some(v) = value {
            // Sanity-check: only accept factors between 0.5 and 3.0
            if v >= 0.5 && v <= 3.0 {
                out.push(RateFactor {
                    name: format!("kb_factor_{i}"),
                    value: v,
                    description: result.title.clone(),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uae_auto_request() -> PremiumRequest {
        PremiumRequest {
            insurance_type: InsuranceType::Auto,
            country: "AE".to_string(),
            state_code: Some("DU".to_string()),
            coverage_amount: 100_000.0,
            customer_age: Some(35),
            vehicle_value: Some(80_000.0),
        }
    }

    #[test]
    fn uae_requests_use_aed() {
        assert_eq!(currency_for_country("AE"), Currency::Aed);
        assert_eq!(currency_for_country("ae"), Currency::Aed);
    }

    #[test]
    fn uae_auto_has_aed_specific_base_rate() {
        let (base_rate, factors) = default_rates(&uae_auto_request());
        assert_eq!(base_rate, 1500.0);
        assert!(!factors.is_empty());
    }
}
