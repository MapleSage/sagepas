use async_trait::async_trait;
use domain::insurance::PremiumCalculation;
use serde::{Deserialize, Serialize};

/// Carrier-specific native PAS rating request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingRequest {
    /// e.g., "travelers"
    pub carrier_id: String,
    /// e.g., "HO3"
    pub product_id: String,
    /// e.g., "TX"
    pub state: String,
    /// Canonical risk data from AI extraction.
    /// Keep this generic now; typed canonical-risk crate can be introduced later.
    pub risk: serde_json::Value,
}

/// Underwriting decision from a rating provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingDecision {
    /// Full quote returned
    Quoted,
    /// Needs manual underwriter review
    Referred,
    /// Risk declined by carrier
    Declined,
    /// Technical error during rating
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Pas,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderIdentity {
    pub id: String,
    pub version: String,
    pub execution_mode: ExecutionMode,
}

/// Canonical rating result - what all rating providers return.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingResult {
    pub decision: RatingDecision,
    pub premium: Option<PremiumCalculation>,
    pub referral_reason: Option<String>,
    pub carrier: String,
    pub product: String,
    pub provider: ProviderIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingError {
    NotSupported { provider: String, reason: String },
    MappingFailed(String),
    ExecutionFailed(String),
    Timeout(u64),
}

/// Rating provider trait - every rater implements this.
/// Excel, Carrier API, Guidewire, Duck Creek, Native - all converge here.
#[async_trait]
pub trait RatingProvider: Send + Sync {
    async fn rate(&self, request: RatingRequest) -> Result<RatingResult, RatingError>;
    fn supports(&self, request: &RatingRequest) -> bool;
    fn identity(&self) -> ProviderIdentity;
}
