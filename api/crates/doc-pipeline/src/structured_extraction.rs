//! Ported from sagesure-us's `fnol-du-adapter::structured_extraction` --
//! GPT + schema-constrained extraction from CU's markdown/fields output.
//! The five schemas are the real `.model_json_schema()` output from the
//! original Pydantic classes (see sagesure-us's own copy for full
//! provenance), not hand-written.

use base64::Engine;
use infra::openai::OpenAIClient;
use serde_json::{Value, json};
use thiserror::Error;

use crate::Domain;

const LIFE_INSURANCE_SCHEMA: &str = include_str!("../schemas/life_insurance.json");
const HEALTH_INSURANCE_SCHEMA: &str = include_str!("../schemas/health_insurance.json");
const PROPERTY_CLAIM_SCHEMA: &str = include_str!("../schemas/property_claim.json");
const PROPERTY_UW_SCHEMA: &str = include_str!("../schemas/property_uw_submission.json");
const AUTO_FNOL_SCHEMA: &str = include_str!("../schemas/auto_fnol.json");
const MARINE_CARGO_FNOL_SCHEMA: &str = include_str!("../schemas/marine_cargo_fnol.json");

#[derive(Debug, Error)]
pub enum StructuredExtractionError {
    #[error("GPT call failed: {0}")]
    GptFailed(String),
    #[error("no content in GPT response")]
    NoContent,
    #[error("response was not valid JSON matching the schema: {0}")]
    InvalidJson(String),
}

/// Maps a coarse line-of-business key to the matching real schema.
/// Domain-aware for "property"/"home" specifically: a FNOL submission is a
/// claim against an existing policy (loss/damage details --
/// `PropertyLossDamageClaimForm`), an underwriting submission is a
/// prospective risk being proposed for coverage (applicant, coverages
/// requested, per-location construction/occupancy/protection detail --
/// `CommercialPropertyUwSubmission`, ported from a real commercial property
/// application, not a generically-imagined one). Using the claims schema
/// for a UW upload produced a mostly-null extraction against a genuine
/// underwriting document -- confirmed live, not assumed. Other lines
/// (life/health/auto/marine) share one schema across both domains for
/// now; life_insurance.json's actual fields turned out UW-submission-
/// relevant despite its "claim or policy document" naming (verified
/// against a real life_submission.pdf), auto_fnol/marine_cargo_fnol are
/// named FNOL-only and may have the same gap property did -- not
/// confirmed either way, flagged rather than assumed fixed.
pub fn schema_for_key(domain: Domain, key: &str) -> Option<&'static str> {
    match key {
        "life" => Some(LIFE_INSURANCE_SCHEMA),
        "health" => Some(HEALTH_INSURANCE_SCHEMA),
        "property" | "home" => Some(match domain {
            Domain::Underwriting => PROPERTY_UW_SCHEMA,
            Domain::Fnol => PROPERTY_CLAIM_SCHEMA,
        }),
        "auto" | "motor" => Some(AUTO_FNOL_SCHEMA),
        "marine" => Some(MARINE_CARGO_FNOL_SCHEMA),
        _ => None,
    }
}

fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Model used for extraction specifically, independent of whatever the
/// caller's default deployment is. Deliberate: reasoning-tier deployments
/// (e.g. `gpt-5.6-luna`, this workspace's default) reject logprobs outright
/// -- confirmed live against this exact Azure OpenAI resource. `gpt-4.1` is
/// deployed on the same resource and returns them, which the Evaluate stage
/// needs for its GPT-side confidence half. Not configurable via `infra`'s
/// `AppConfig` because this is an extraction-quality decision, not a
/// deployment/environment one -- every environment should use the same
/// model for this specific call.
const EXTRACTION_MODEL: &str = "gpt-4.1";

/// Result of one Map-stage GPT call: the parsed structured fields, the raw
/// text GPT returned (needed to locate each field's token span for
/// confidence), and the per-token logprobs themselves.
pub struct MapResult {
    pub fields: Value,
    pub generated_text: String,
    pub logprobs: Vec<Value>,
}

/// Calls GPT with the real field schema injected into the system prompt,
/// forcing schema-constrained structured output. `page_images` are raw PNG
/// (or the original image bytes for a direct image upload) -- reference
/// parity requires these reach the model, not just CU's markdown text: the
/// FNOL accelerator's Map step (`map_handler.py`) explicitly rasterizes
/// every PDF page via `pdf2image` and attaches each one as an image part
/// alongside the markdown, on the reasoning that CU's OCR text and GPT's
/// own reading of the page image are two independent signals worth cross-
/// checking (see the Evaluate stage, which merges their confidences) -- a
/// text-only call can't produce the second one.
pub async fn extract_structured_fields(
    openai: &OpenAIClient,
    markdown: &str,
    cu_fields: Option<&Value>,
    schema_json: &str,
    page_images: &[Vec<u8>],
) -> Result<MapResult, StructuredExtractionError> {
    let schema_value: Value = serde_json::from_str(schema_json)
        .map_err(|e| StructuredExtractionError::InvalidJson(format!("embedded schema: {e}")))?;
    let schema_compact = serde_json::to_string(&schema_value).unwrap_or_default();

    let markdown_snippet = safe_truncate(markdown, 12000);
    let cu_fields_str = cu_fields
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
        .unwrap_or_else(|| "(none)".to_string());

    let system_prompt = format!(
        r#"You are an AI assistant that extracts data from insurance documents.
You must ALWAYS return valid JSON matching the schema below — never return plain text.
If a field value cannot be determined from the document or image, set it to null.
You must return ONLY valid JSON that matches this exact schema:
{schema_compact}"#
    );

    let text_part = json!({
        "type": "input_text",
        "text": format!(
            "Document content (markdown):\n{markdown_snippet}\n\nAdditional extracted fields (if any):\n{cu_fields_str}"
        ),
    });

    let mut user_content: Vec<Value> = vec![text_part];
    for image_bytes in page_images {
        let encoded = base64::engine::general_purpose::STANDARD.encode(image_bytes.as_slice());
        user_content.push(json!({
            "type": "input_image",
            "image_url": format!("data:image/png;base64,{encoded}"),
        }));
    }

    let messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_content}),
    ];

    tracing::info!(
        page_image_count = page_images.len(),
        "sending page images to GPT extraction (reference-parity Map stage)"
    );

    let (content, logprobs) = openai
        .chat_completion_with_logprobs(&messages, EXTRACTION_MODEL, 5, Some(4096))
        .await
        .map_err(|e| StructuredExtractionError::GptFailed(e.to_string()))?;

    let cleaned = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let fields = serde_json::from_str(cleaned)
        .map_err(|e| StructuredExtractionError::InvalidJson(format!("{e}: {cleaned}")))?;

    Ok(MapResult {
        fields,
        generated_text: content,
        logprobs,
    })
}
