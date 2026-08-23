//! Ported from sagesure-us's `fnol-du-adapter::structured_extraction` --
//! GPT + schema-constrained extraction from CU's markdown/fields output.
//! The five schemas are the real `.model_json_schema()` output from the
//! original Pydantic classes (see sagesure-us's own copy for full
//! provenance), not hand-written.

use infra::openai::OpenAIClient;
use serde_json::{Value, json};
use thiserror::Error;

const LIFE_INSURANCE_SCHEMA: &str = include_str!("../schemas/life_insurance.json");
const HEALTH_INSURANCE_SCHEMA: &str = include_str!("../schemas/health_insurance.json");
const PROPERTY_CLAIM_SCHEMA: &str = include_str!("../schemas/property_claim.json");
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
pub fn schema_for_key(key: &str) -> Option<&'static str> {
    match key {
        "life" => Some(LIFE_INSURANCE_SCHEMA),
        "health" => Some(HEALTH_INSURANCE_SCHEMA),
        "property" | "home" => Some(PROPERTY_CLAIM_SCHEMA),
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

/// Calls GPT with the real field schema injected into the system prompt,
/// forcing schema-constrained structured output.
pub async fn extract_structured_fields(
    openai: &OpenAIClient,
    markdown: &str,
    cu_fields: Option<&Value>,
    schema_json: &str,
) -> Result<Value, StructuredExtractionError> {
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
If a field value cannot be determined from the document, set it to null.
You must return ONLY valid JSON that matches this exact schema:
{schema_compact}"#
    );

    let user_content = format!(
        "Document content (markdown):\n{markdown_snippet}\n\nAdditional extracted fields (if any):\n{cu_fields_str}"
    );

    let messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_content}),
    ];

    let response = openai
        .chat_completion(&messages, "", Some(0.1), Some(4096))
        .await
        .map_err(|e| StructuredExtractionError::GptFailed(e.to_string()))?;

    let content = response
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .ok_or(StructuredExtractionError::NoContent)?;

    let cleaned = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str(cleaned)
        .map_err(|e| StructuredExtractionError::InvalidJson(format!("{e}: {cleaned}")))
}
