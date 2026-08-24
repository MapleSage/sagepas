//! Ported from sagesure-us's `fnol-du-adapter::content_understanding`, with
//! two deliberate fixes over that version:
//!
//! 1. That version routed images to `prebuilt-imageSearch` (an
//!    image-similarity analyzer, not a field extractor) -- there was no CU
//!    limitation forcing that branch, just the wrong analyzer choice.
//!
//! 2. Analyzer choice corrected again, live-tested against this exact CU
//!    resource (work order Step 2 investigation): `prebuilt-document`
//!    returns full `markdown` but zero `pages[].words`/`pages[].lines` --
//!    confirmed via a raw analyze call (`page1: words=0 lines=0`). Neither
//!    analyzer returns schema-bound `fields` without a custom analyzer
//!    template, so that's not the differentiator. `prebuilt-layout` (the
//!    classic Document Intelligence layout model CU also exposes) returns
//!    the *identical* markdown (confirmed byte-identical length) plus real
//!    per-word confidence and position (`page1: words=494 lines=167`,
//!    first word "LIFE" conf=0.996) -- strictly more data for the same
//!    markdown, which is what the citation-grounding primitive
//!    (`evaluate.rs`) needs and `prebuilt-document` cannot supply. This
//!    client always uses `prebuilt-layout`, for every input type.
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
pub struct CuSpan {
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuWord {
    pub content: String,
    pub span: CuSpan,
    pub confidence: f64,
    #[serde(default)]
    pub polygon: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuLine {
    pub content: String,
    pub span: CuSpan,
    #[serde(default)]
    pub polygon: Option<Vec<f64>>,
}

/// Matches the reference (`azure_helper/model/content_understanding.py`
/// `Page`) closely enough to reuse its line/word confidence-derivation
/// approach: CU gives per-*word* confidence, not per-line, so a line's
/// confidence is derived (min of its contained words) -- see `evaluate.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuPage {
    #[serde(rename = "pageNumber")]
    pub page_number: i32,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub words: Vec<CuWord>,
    #[serde(default)]
    pub lines: Vec<CuLine>,
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
    /// Per-page word/line position + confidence data. `prebuilt-layout`
    /// populates this; `prebuilt-document` does not (confirmed live --
    /// `page1: words=0 lines=0` for the identical document). `default`
    /// rather than a hard parse failure since other analyzer configs may
    /// still omit it.
    #[serde(default)]
    pub pages: Vec<CuPage>,
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

/// Every content type -- PDF or image -- uses the same analyzer. There is
/// no format-based reason to route images differently: Document
/// Intelligence's layout model accepts image inputs directly, same as PDFs
/// (per Azure's own multi-format support for `prebuilt-layout`; not
/// independently re-verified against an image input in this session --
/// only the PDF path has live evidence here, see `content_understanding.rs`
/// module docs).
pub fn select_analyzer(_content_type: &str) -> &'static str {
    "prebuilt-layout"
}
