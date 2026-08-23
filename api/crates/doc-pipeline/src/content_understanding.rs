//! Ported from sagesure-us's `fnol-du-adapter::content_understanding`, with
//! one deliberate fix: that version routed images to `prebuilt-imageSearch`
//! (an image-similarity analyzer, not a field extractor), which is why image
//! ingestion there is accepted but produces meaningfully worse structured
//! output than PDFs. CU's `prebuilt-document` analyzer accepts image inputs
//! (JPEG/PNG/BMP/TIFF/HEIF) directly and does real OCR+layout extraction on
//! them the same as PDFs -- there was no CU limitation forcing the image
//! branch, just the wrong analyzer choice. This client always uses
//! `prebuilt-document`, for every input type. See work order Phase 1 /
//! plan file `abstract-yawning-pinwheel.md`.
//!
//! Also carries the `REQUEST_TIMEOUT_SECONDS`-equivalent fix noted in
//! `FREEZE-INSTRUCTION-2026-08-11.sh`'s freeze-time gap analysis: the
//! sagesure-us Rust client lacked a request timeout the standalone Python
//! pipeline had. `analyze_bytes`/`analyze_blob_url` here bound every
//! individual HTTP request (not just the overall poll loop), same as
//! sagesure-us's copy already does -- carried forward, not reintroducing
//! the gap the freeze note flagged on the *other* side (base64 fallback on
//! CU 400s, which lives in the ingest stage in `lib.rs`, not here).

use azure_core::auth::TokenCredential;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CuError {
    #[error("CU request failed: {0}")]
    RequestFailed(String),
    #[error("CU polling timed out after {0}s")]
    Timeout(u64),
    #[error("CU analysis failed: {0}")]
    AnalysisFailed(String),
    #[error("CU auth token acquisition failed: {0}")]
    AuthFailed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuDocumentContent {
    pub markdown: String,
    pub kind: String,
    #[serde(rename = "startPageNumber")]
    pub start_page_number: Option<i32>,
    #[serde(rename = "endPageNumber")]
    pub end_page_number: Option<i32>,
    pub fields: Option<Value>,
    #[serde(rename = "analyzerId")]
    pub analyzer_id: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuResultData {
    #[serde(rename = "analyzerId")]
    pub analyzer_id: String,
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub warnings: Vec<Value>,
    pub contents: Vec<CuDocumentContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuAnalyzedResult {
    pub id: String,
    pub status: String,
    pub result: CuResultData,
}

#[derive(Debug, Clone)]
pub struct ContentUnderstandingClient {
    http: Client,
    endpoint: String,
    api_version: String,
}

/// Acquire a bearer token for Cognitive Services scope via workload identity.
pub async fn acquire_cu_token() -> Result<String, CuError> {
    let cred = azure_identity::DefaultAzureCredential::create(
        azure_identity::TokenCredentialOptions::default(),
    )
    .map_err(|e| CuError::AuthFailed(format!("credential init: {e}")))?;
    let token = cred
        .get_token(&["https://cognitiveservices.azure.com/.default"])
        .await
        .map_err(|e| CuError::AuthFailed(format!("token acquisition: {e}")))?;
    Ok(token.token.secret().to_string())
}

impl ContentUnderstandingClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        // reqwest::Client::new() has no default timeout -- a single stalled
        // request hangs forever, bypassing the poll-loop's own timeout bound
        // (only checked between iterations). Bound each request here.
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_version: "2025-11-01".to_string(),
        }
    }

    fn analyze_url(&self, analyzer_id: &str) -> String {
        format!(
            "{}/contentunderstanding/analyzers/{}:analyze?api-version={}",
            self.endpoint, analyzer_id, self.api_version
        )
    }

    pub async fn analyze_bytes(
        &self,
        analyzer_id: &str,
        file_bytes: &[u8],
        bearer_token: &str,
        timeout_secs: u64,
    ) -> Result<CuAnalyzedResult, CuError> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(file_bytes);
        let payload = json!({ "inputs": [{"data": encoded}] });

        let resp = self
            .http
            .post(&self.analyze_url(analyzer_id))
            .header("Authorization", format!("Bearer {}", bearer_token))
            .header("Content-Type", "application/json")
            .header("x-ms-useragent", "sagepas/rust-cu-client")
            .json(&payload)
            .send()
            .await
            .map_err(|e| CuError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CuError::RequestFailed(format!("HTTP {} — {}", status, body)));
        }

        let operation_location = resp
            .headers()
            .get("operation-location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| CuError::AnalysisFailed("missing operation-location header".into()))?;

        tracing::info!(analyzer_id, "CU analysis submitted, polling for result");
        self.poll_result(&operation_location, bearer_token, timeout_secs).await
    }

    async fn poll_result(
        &self,
        operation_url: &str,
        bearer_token: &str,
        timeout_secs: u64,
    ) -> Result<CuAnalyzedResult, CuError> {
        let started = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let mut interval = Duration::from_millis(500);

        loop {
            if started.elapsed() > timeout {
                return Err(CuError::Timeout(timeout_secs));
            }
            tokio::time::sleep(interval).await;

            let resp = self
                .http
                .get(operation_url)
                .header("Authorization", format!("Bearer {}", bearer_token))
                .send()
                .await
                .map_err(|e| CuError::RequestFailed(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(CuError::RequestFailed(format!("poll HTTP {} — {}", status, body)));
            }

            let body: Value = resp
                .json()
                .await
                .map_err(|e| CuError::AnalysisFailed(e.to_string()))?;

            let status = body
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();

            match status.as_str() {
                "succeeded" => {
                    tracing::info!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "CU analysis succeeded"
                    );
                    let result: CuAnalyzedResult = serde_json::from_value(body)
                        .map_err(|e| CuError::AnalysisFailed(format!("parse error: {}", e)))?;
                    return Ok(result);
                }
                "failed" => {
                    let reason = body
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");
                    return Err(CuError::AnalysisFailed(reason.to_string()));
                }
                _ => {
                    interval = (interval * 2).min(Duration::from_secs(4));
                }
            }
        }
    }
}

/// Every content type -- PDF or image -- uses the same real document
/// analyzer. There is no format-based reason to route images differently:
/// `prebuilt-document` does OCR+layout extraction on image inputs directly.
pub fn select_analyzer(_content_type: &str) -> &'static str {
    "prebuilt-document"
}
