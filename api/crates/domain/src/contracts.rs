/// Interface contract for ABDM (Ayushman Bharat Digital Mission) integration.
/// Requires IRDAI registration as a Health Repository Operator — deferred.
/// Trait body intentionally empty; impl ships when the business deal closes.
pub trait AbdmClient: Send + Sync {
    // TODO: fn fetch_health_record(&self, abha_id: &str) -> impl Future<Output = anyhow::Result<HealthRecord>>;
}

/// Interface contract for insurer quote APIs.
/// Requires commercial agreements with 8+ insurers — deferred.
pub trait InsurerQuoteProvider: Send + Sync {
    // TODO: fn get_quotes(&self, request: QuoteRequest) -> impl Future<Output = anyhow::Result<Vec<Quote>>>;
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepfakeResult {
    pub is_fake: bool,
    pub confidence: f32,
    pub model: String,
    pub details: Option<String>,
}

/// Contract for the deepfake detection ML sidecar.
/// Real model wired in the ML track; sidecar returns 503 when unavailable.
pub trait DeepfakeClient: Send + Sync {
    fn analyze_video(
        &self,
        blob_url: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<DeepfakeResult>> + Send + '_>,
    >;
}
