//! KB-grounded scoring -- the mechanism, not the domain rules.
//!
//! The consolidation decision's non-negotiable requirement (2): "Analysis
//! grounded in the knowledge base with the detail retained, not summarized
//! away. Each conclusion carries the KB material it rests on." Neither
//! reference point clears this today -- sagesure-us's risk-scorer only
//! returns a `kb_used: bool`, and even the standalone UW workbench (the
//! reference implementation) truncates retained passage text to 500 chars.
//! This deliberately exceeds both: the full `SearchResult.content` from
//! Azure AI Search is attached to every conclusion, not a summary and not
//! a truncation -- the acceptance bar is more specific than "copy the
//! reference," so it wins over UW's own truncation.
//!
//! Grounding is enforced structurally, not by trusting GPT to cite
//! accurately: GPT is asked which *retrieved result index* supports each
//! conclusion (a small integer), and the full passage text is then attached
//! programmatically from what was actually retrieved -- never from GPT's
//! paraphrase of it.
//!
//! What stays domain-specific and lives in the caller (`handlers/fnol.rs`/
//! `handlers/uw.rs`), per the "one engine, two configs" split: the search
//! query built from extracted fields, the `domain_prompt` framing (severity
//! assessment vs underwriting appetite), and what a conclusion's confidence
//! level means for routing (STP / review / decline). This module only
//! searches, grounds, and returns -- it doesn't decide.

use infra::openai::OpenAIClient;
use infra::search::{SearchClient, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KbScoringError {
    #[error("KB search failed: {0}")]
    SearchFailed(String),
    #[error("GPT reasoning call failed: {0}")]
    GptFailed(String),
    #[error("GPT response was not valid JSON: {0}")]
    InvalidJson(String),
}

/// Full grounding material for one conclusion -- the actual retrieved
/// passage, not a summary of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbCitation {
    pub title: Option<String>,
    pub source: Option<String>,
    pub passage_text: String,
    pub relevance_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundedConclusion {
    pub statement: String,
    /// Empty when GPT found no retrieved passage actually supports the
    /// statement -- surfaced as-is rather than silently dropped, since an
    /// ungrounded conclusion is exactly the failure mode this exists to
    /// catch, not hide.
    pub citations: Vec<KbCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbScoringResult {
    pub conclusions: Vec<GroundedConclusion>,
    /// How many of the retrieved passages were never cited by any
    /// conclusion -- a caller-visible signal that the KB search may have
    /// been too broad, not swept under the rug.
    pub retrieved_count: usize,
    pub uncited_count: usize,
}

/// Searches `kb_index` with `query`, then asks GPT to produce grounded
/// conclusions using only what was actually retrieved -- `domain_prompt`
/// supplies the caller's framing (what kind of conclusions to draw and
/// about what), this function supplies the grounding discipline.
pub async fn score_with_kb(
    search: &SearchClient,
    openai: &OpenAIClient,
    kb_index: &str,
    query: &str,
    domain_prompt: &str,
    extracted_fields: &Value,
) -> Result<KbScoringResult, KbScoringError> {
    let results: Vec<SearchResult> = search
        .search_kb(kb_index, query, None, 5)
        .await
        .map_err(|e| KbScoringError::SearchFailed(e.to_string()))?;

    if results.is_empty() {
        // No KB material at all -- return no conclusions rather than let
        // GPT invent ungrounded ones. An empty result is a real, visible
        // signal (retrieved_count: 0), not a silent GPT free-association.
        return Ok(KbScoringResult {
            conclusions: Vec::new(),
            retrieved_count: 0,
            uncited_count: 0,
        });
    }

    let passages_for_prompt: Vec<Value> = results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            json!({
                "index": i,
                "title": r.title,
                "source": r.source,
                "content": r.content,
            })
        })
        .collect();

    let system_prompt = format!(
        r#"{domain_prompt}

You must ground every conclusion in the retrieved knowledge-base passages provided below.
For each conclusion, cite the index (or indices) of the passage(s) that actually support it.
If no retrieved passage supports a conclusion you would otherwise draw, do not draw it.
Return ONLY valid JSON matching this shape, nothing else:
{{"conclusions": [{{"statement": "...", "citation_indices": [0, 2]}}]}}"#
    );

    let user_content = format!(
        "Extracted fields:\n{}\n\nRetrieved knowledge-base passages:\n{}",
        serde_json::to_string_pretty(extracted_fields).unwrap_or_default(),
        serde_json::to_string_pretty(&passages_for_prompt).unwrap_or_default(),
    );

    let messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_content}),
    ];

    let response = openai
        .chat_completion(&messages, "", Some(0.1), Some(2048))
        .await
        .map_err(|e| KbScoringError::GptFailed(e.to_string()))?;

    let content = response
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| KbScoringError::GptFailed("no content in GPT response".to_string()))?;

    let cleaned = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    #[derive(Deserialize)]
    struct RawConclusion {
        statement: String,
        #[serde(default)]
        citation_indices: Vec<usize>,
    }
    #[derive(Deserialize)]
    struct RawResponse {
        conclusions: Vec<RawConclusion>,
    }

    let parsed: RawResponse = serde_json::from_str(cleaned)
        .map_err(|e| KbScoringError::InvalidJson(format!("{e}: {cleaned}")))?;

    let mut cited_indices = std::collections::HashSet::new();
    let conclusions = parsed
        .conclusions
        .into_iter()
        .map(|rc| {
            let citations = rc
                .citation_indices
                .iter()
                .filter_map(|&i| {
                    cited_indices.insert(i);
                    results.get(i).map(|r| KbCitation {
                        title: r.title.clone(),
                        source: r.source.clone(),
                        // The actual retrieved passage, verbatim -- never
                        // GPT's paraphrase, never truncated.
                        passage_text: r.content.clone(),
                        relevance_score: r.score,
                    })
                })
                .collect();
            GroundedConclusion {
                statement: rc.statement,
                citations,
            }
        })
        .collect();

    let uncited_count = results.len() - cited_indices.len();

    Ok(KbScoringResult {
        conclusions,
        retrieved_count: results.len(),
        uncited_count,
    })
}
