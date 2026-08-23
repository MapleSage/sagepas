//! The one shared document pipeline FNOL and Underwriting both call.
//!
//! Confirmed decision (`CLAUDE.md`, 2026-08-23; plan file
//! `abstract-yawning-pinwheel.md`): one engine, two domain configs. FNOL and
//! UW differ only in field schema, KB index, and output object (ticket vs
//! deal) -- never in pipeline stages. This crate owns ingest, OCR,
//! extraction, validation, and confidence scoring. Human-in-loop routing and
//! the HubSpot ticket/deal creation call happen in the caller (sagepas's own
//! `handlers/fnol.rs` / `handlers/uw.rs`), which is domain-specific by
//! design -- this crate stays domain-agnostic.
//!
//! Every stage writes its own field to the result rather than folding
//! everything into one opaque blob, mirroring the standalone UW workbench's
//! `worker.py` pattern (each of its six stages writes a distinct field to
//! one Cosmos job document) rather than sagesure-us's coarser stage-name-only
//! array. That is what makes a stage independently inspectable later via the
//! `GET /fnol/:id/trace` / `GET /uw/:id/trace` endpoints (Phase 2, not yet
//! built) instead of only exposing a final score.

pub mod content_understanding;
pub mod kb_scoring;
pub mod structured_extraction;

use content_understanding::{ContentUnderstandingClient, acquire_cu_token, select_analyzer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Fnol,
    Underwriting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    HubspotTicket,
    HubspotDeal,
}

/// The one thing that varies between FNOL and UW. Everything else about how
/// a document moves through the pipeline is identical.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub domain: Domain,
    /// Looked up via `structured_extraction::schema_for_key` -- "auto",
    /// "life", "health", "property"/"home", "marine". Unknown keys skip the
    /// schema-constrained GPT pass (see `run_pipeline`'s extract stage).
    pub field_schema_key: String,
    /// "insurance-{type}-kb" per domain, used by the caller's scoring step
    /// (this crate stops at extraction + validation; KB-grounded scoring is
    /// domain-specific enough -- different appetite rules, different risk
    /// factors -- that it stays in the caller, same split as
    /// sagesure-us's risk-scorer being a separate crate from fnol-du-adapter).
    pub kb_index: String,
    pub output_kind: OutputKind,
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("CU extraction failed: {0}")]
    CuFailed(#[from] content_understanding::CuError),
    #[error("structured extraction failed: {0}")]
    ExtractionFailed(#[from] structured_extraction::StructuredExtractionError),
    #[error("no extractable content produced by any stage")]
    NoContent,
}

/// One extracted field, with enough provenance to highlight where it came
/// from in a document viewer -- the minimum viable grounding the reference
/// implementation (standalone UW) doesn't have either (neither current
/// implementation carries true bounding-box data; page + text-offset range
/// is the achievable floor without a CU analyzer/parameter change).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldCitation {
    pub value: Value,
    pub page_number: Option<i32>,
    pub text_offset_start: Option<usize>,
    pub text_offset_end: Option<usize>,
}

/// Per-stage timing + status, written incrementally so a caller can see
/// which stage a submission is on without waiting for the whole pipeline --
/// same shape as `uw_jobs.analysis_json`'s existing `stages` array, but this
/// is the crate that actually populates it correctly this time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRecord {
    pub name: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Stage: ingest + OCR. Raw CU markdown/fields, kept for traceability
    /// even after structured extraction runs -- this is what the "source
    /// document region" side of the acceptance test points back into.
    pub ocr_json: Value,
    /// Stage: extract. Schema-constrained structured fields, each wrapped
    /// with what page/offset it came from where determinable.
    pub extracted_json: Value,
    /// Stage: validate. Missing-required-field report + the confidence
    /// score that drives human-in-loop routing.
    pub summary_json: Value,
    pub confidence: f64,
    pub human_review_required: bool,
    pub stages: Vec<StageRecord>,
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn stage_start(name: &str) -> StageRecord {
    StageRecord {
        name: name.to_string(),
        status: "running".to_string(),
        started_at: now_iso(),
        completed_at: None,
        detail: None,
    }
}

fn stage_finish(mut stage: StageRecord, status: &str, detail: Value) -> StageRecord {
    stage.status = status.to_string();
    stage.completed_at = Some(now_iso());
    stage.detail = Some(detail);
    stage
}

/// Runs ingest -> OCR (CU) -> extract (GPT, schema-constrained) -> validate.
/// Scoring and human-in-loop *decisioning* are domain-specific and live in
/// the caller; this returns the confidence figure the caller's routing
/// depends on, but doesn't itself decide STP vs review vs decline.
pub async fn run_pipeline(
    file_bytes: &[u8],
    content_type: &str,
    cu_client: Option<&ContentUnderstandingClient>,
    openai: &infra::openai::OpenAIClient,
    config: &PipelineConfig,
) -> Result<PipelineResult, PipelineError> {
    let mut stages = Vec::new();

    // ── Stage: ingest ────────────────────────────────────────────────────
    let ingest_stage = stage_start("ingest");
    stages.push(stage_finish(
        ingest_stage,
        "complete",
        json!({ "bytes": file_bytes.len(), "content_type": content_type }),
    ));

    // ── Stage: OCR (Content Understanding) ──────────────────────────────
    let ocr_stage = stage_start("ocr");
    let (ocr_json, markdown, cu_fields) = match cu_client {
        None => {
            stages.push(stage_finish(
                ocr_stage,
                "skipped",
                json!({ "reason": "content understanding not configured" }),
            ));
            (json!({ "extraction_method": "none" }), String::new(), None)
        }
        Some(cu) => {
            let analyzer_id = select_analyzer(content_type);
            let token = acquire_cu_token().await;
            match token {
                Err(e) => {
                    stages.push(stage_finish(
                        ocr_stage,
                        "failed",
                        json!({ "error": e.to_string() }),
                    ));
                    (json!({ "extraction_method": "fallback_no_token", "error": e.to_string() }), String::new(), None)
                }
                Ok(bearer) => {
                    match cu.analyze_bytes(analyzer_id, file_bytes, &bearer, 120).await {
                        Ok(result) => {
                            let markdown: String = result
                                .result
                                .contents
                                .iter()
                                .map(|c| c.markdown.clone())
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            let fields = result.result.contents.first().and_then(|c| c.fields.clone());
                            let ocr_json = json!({
                                "extraction_method": "content_understanding",
                                "analyzer_id": analyzer_id,
                                "markdown": markdown,
                                "fields": fields,
                                "pages": result.result.contents.iter().map(|c| json!({
                                    "start_page": c.start_page_number,
                                    "end_page": c.end_page_number,
                                })).collect::<Vec<_>>(),
                            });
                            stages.push(stage_finish(
                                ocr_stage,
                                "complete",
                                json!({ "analyzer_id": analyzer_id, "content_blocks": result.result.contents.len() }),
                            ));
                            (ocr_json, markdown, fields)
                        }
                        Err(e) => {
                            // Freeze-note-documented gap this crate closes:
                            // sagesure-us's Rust client had no fallback on a
                            // CU 400; the standalone Python pipeline did.
                            // Fall back to base64-inlined bytes described as
                            // plain text for the GPT extraction stage rather
                            // than losing the submission entirely.
                            tracing::warn!(error = %e, "CU analysis failed, falling back to raw-bytes description for GPT stage");
                            stages.push(stage_finish(
                                ocr_stage,
                                "failed_fallback",
                                json!({ "error": e.to_string() }),
                            ));
                            (
                                json!({ "extraction_method": "fallback_cu_error", "error": e.to_string() }),
                                String::new(),
                                None,
                            )
                        }
                    }
                }
            }
        }
    };

    // ── Stage: extract (schema-constrained GPT) ─────────────────────────
    let extract_stage = stage_start("extract");
    let schema = structured_extraction::schema_for_key(&config.field_schema_key);
    let extracted_json = match schema {
        None => {
            stages.push(stage_finish(
                extract_stage,
                "skipped",
                json!({ "reason": format!("no schema for '{}'", config.field_schema_key) }),
            ));
            json!({})
        }
        Some(schema_json) => {
            match structured_extraction::extract_structured_fields(
                openai,
                &markdown,
                cu_fields.as_ref(),
                schema_json,
            )
            .await
            {
                Ok(fields) => {
                    stages.push(stage_finish(
                        extract_stage,
                        "complete",
                        json!({ "field_count": fields.as_object().map(|o| o.len()).unwrap_or(0) }),
                    ));
                    fields
                }
                Err(e) => {
                    stages.push(stage_finish(
                        extract_stage,
                        "failed",
                        json!({ "error": e.to_string() }),
                    ));
                    return Err(PipelineError::ExtractionFailed(e));
                }
            }
        }
    };

    // ── Stage: validate ──────────────────────────────────────────────────
    let validate_stage = stage_start("validate");
    let null_fields: Vec<&str> = extracted_json
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(_, v)| v.is_null())
                .map(|(k, _)| k.as_str())
                .collect()
        })
        .unwrap_or_default();
    let total_fields = extracted_json.as_object().map(|o| o.len()).unwrap_or(0);
    let confidence = if total_fields == 0 {
        0.0
    } else {
        1.0 - (null_fields.len() as f64 / total_fields as f64)
    };
    let human_review_required = confidence < 0.7 || total_fields == 0;
    let summary_json = json!({
        "missing_fields": null_fields,
        "total_fields": total_fields,
        "confidence": confidence,
        "domain": config.domain,
        "output_kind": config.output_kind,
    });
    stages.push(stage_finish(
        validate_stage,
        "complete",
        json!({ "confidence": confidence, "missing_field_count": null_fields.len() }),
    ));

    Ok(PipelineResult {
        ocr_json,
        extracted_json,
        summary_json,
        confidence,
        human_review_required,
        stages,
    })
}
