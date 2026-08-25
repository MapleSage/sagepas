//! Evaluate stage: merges CU's own per-field confidence (or, when the
//! analyzer didn't return custom fields, line-matched confidence derived
//! from `pages[].words[].confidence`) with GPT's log-prob-derived
//! confidence into one entity_score, and builds the page/offset citation
//! for each field along the way.
//!
//! Ported logic, not guessed: `content_understanding_confidence_evaluator.py`
//! (line-matching + min-of-contained-words), `openai_confidence_evaluator.py`
//! (token-logprob -> confidence via exp(avg_logprob)), and `confidence.py`'s
//! `merge_confidence_values` (per-field min() of the two valid signals).
//! GPT logprobs require a non-reasoning deployment -- confirmed live,
//! `gpt-5.6-luna` rejects them outright ("logprobs are not supported with
//! reasoning models"); `gpt-4.1` on the same resource returns them. Callers
//! pass `gpt-4.1` for this stage specifically (see `structured_extraction.rs`).

use crate::FieldCitation;
use crate::content_understanding::CuPage;
use serde_json::Value;

#[derive(Debug, Clone)]
struct DiLine {
    content: String,
    page_number: i32,
    confidence: f64,
    offset: i64,
    length: i64,
}

fn extract_lines(pages: &[CuPage]) -> Vec<DiLine> {
    let mut out = Vec::new();
    for page in pages {
        for line in &page.lines {
            let line_start = line.span.offset;
            let line_end = line.span.offset + line.span.length;
            let contained: Vec<f64> = page
                .words
                .iter()
                .filter(|w| w.span.offset >= line_start && w.span.offset + w.span.length <= line_end)
                .map(|w| w.confidence)
                .collect();
            let confidence = if contained.is_empty() {
                0.0
            } else {
                contained.iter().cloned().fold(f64::INFINITY, f64::min)
            };
            out.push(DiLine {
                content: line.content.clone(),
                page_number: page.page_number,
                confidence,
                offset: line.span.offset,
                length: line.span.length,
            });
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Exact match first, falling back to substring containment -- same two-pass
/// strategy as the reference (`value_match` then `value_contains`).
fn find_matching_lines<'a>(value: &str, lines: &'a [DiLine]) -> Vec<&'a DiLine> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    let needle = normalize(value);
    let exact: Vec<&DiLine> = lines.iter().filter(|l| normalize(&l.content) == needle).collect();
    if !exact.is_empty() {
        return exact;
    }
    lines.iter().filter(|l| normalize(&l.content).contains(&needle)).collect()
}

/// CU-side confidence + citation for one field value: search page lines for
/// a match, take the min confidence among matches (mirrors the reference's
/// `min` resolver), cite the first match's page + character offsets.
fn cu_field_confidence(value: &Value, lines: &[DiLine]) -> (f64, Option<FieldCitation>) {
    let value_str = match value {
        Value::Null => return (0.0, None),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let matches = find_matching_lines(&value_str, lines);
    if matches.is_empty() {
        return (0.0, None);
    }
    let confidence = matches
        .iter()
        .map(|l| l.confidence)
        .fold(f64::INFINITY, f64::min);
    let first = matches[0];
    let citation = FieldCitation {
        value: value.clone(),
        page_number: Some(first.page_number),
        text_offset_start: Some(first.offset.max(0) as usize),
        text_offset_end: Some((first.offset + first.length).max(0) as usize),
    };
    (confidence, Some(citation))
}

/// GPT-side confidence for one field value from the raw generated JSON text
/// and its per-token logprobs. Unlike the reference (which re-tokenizes with
/// tiktoken to recover character offsets), the Responses API's logprobs
/// array already carries each token's literal string in emission order, so
/// offsets are just a running sum over that -- no external tokenizer needed.
fn gpt_field_confidence(value: &Value, generated_text: &str, logprobs: &[Value]) -> f64 {
    let value_str = match value {
        Value::Null => return 0.0,
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if value_str.is_empty() {
        return 0.0;
    }

    // Build (start, end, logprob) per token by walking the token strings in
    // order and accumulating character offsets into generated_text.
    let mut token_spans: Vec<(usize, usize, f64)> = Vec::with_capacity(logprobs.len());
    let mut pos = 0usize;
    for entry in logprobs {
        let token = entry.get("token").and_then(|t| t.as_str()).unwrap_or("");
        let logprob = entry.get("logprob").and_then(|l| l.as_f64()).unwrap_or(f64::NEG_INFINITY);
        let start = pos;
        let end = pos + token.len();
        token_spans.push((start, end, logprob));
        pos = end;
    }

    let Some(start_index) = generated_text.find(&value_str) else {
        return 0.0;
    };
    let end_index = start_index + value_str.len();

    let overlapping: Vec<f64> = token_spans
        .iter()
        .filter(|(s, e, _)| *s < end_index && *e > start_index)
        .map(|(_, _, lp)| *lp)
        .filter(|lp| *lp > -9999.0)
        .collect();

    if overlapping.is_empty() {
        return 0.0;
    }

    let avg_logprob = overlapping.iter().sum::<f64>() / overlapping.len() as f64;
    avg_logprob.exp().clamp(0.0, 1.0)
}

/// Per-field min() of whichever signals are actually present -- exact merge
/// rule as `confidence.py::merge_field_confidence_value`'s scalar branch.
fn merge(cu: f64, gpt: f64) -> f64 {
    let valid: Vec<f64> = [cu, gpt].into_iter().filter(|v| *v != 0.0).collect();
    if valid.is_empty() {
        0.0
    } else {
        valid.iter().cloned().fold(f64::INFINITY, f64::min)
    }
}

pub struct EvaluateResult {
    /// Aggregate of per-field merged (CU + GPT) confidence -- "was the text
    /// read correctly." New in this stage; the reference's real definition,
    /// not a null-density proxy.
    pub entity_score: f64,
    /// Unchanged from the pipeline's prior single `confidence` figure
    /// (fraction of non-null fields) -- kept as-is per work order Step 3,
    /// just renamed honestly rather than replaced.
    pub schema_score: f64,
    /// `extracted_json`, each non-null scalar field replaced with
    /// `{value, confidence, page_number, text_offset_start, text_offset_end}`.
    pub cited_json: Value,
    /// How many non-null fields got a real page/offset citation, out of how
    /// many were scored -- work order Step 2's sign-off condition: an
    /// uncited field must be countable and surfaced, not silently absent.
    pub cited_count: usize,
    pub scored_count: usize,
}

/// `extracted_json` must be a flat-ish object (schema-shaped); nested
/// objects/arrays are walked recursively so every leaf value gets its own
/// citation, matching the reference's recursive `evaluate_field_value_confidence`.
pub fn evaluate(
    extracted_json: &Value,
    cu_pages: &[CuPage],
    generated_text: &str,
    logprobs: &[Value],
) -> EvaluateResult {
    let lines = extract_lines(cu_pages);
    let mut all_confidences = Vec::new();
    let cited_json = annotate(extracted_json, &lines, generated_text, logprobs, &mut all_confidences);

    let entity_score = if all_confidences.is_empty() {
        0.0
    } else {
        all_confidences.iter().sum::<f64>() / all_confidences.len() as f64
    };

    let (cited_count, scored_count) = count_citations(&cited_json);

    let total_fields = extracted_json.as_object().map(|o| o.len()).unwrap_or(0);
    let null_fields = extracted_json
        .as_object()
        .map(|o| o.values().filter(|v| v.is_null()).count())
        .unwrap_or(0);
    let schema_score = if total_fields == 0 {
        0.0
    } else {
        1.0 - (null_fields as f64 / total_fields as f64)
    };

    EvaluateResult {
        entity_score,
        schema_score,
        cited_json,
        cited_count,
        scored_count,
    }
}

/// Counts leaf value-objects (the `{value, confidence, page_number, ...}`
/// shape `annotate` produces) and how many of those have a real citation.
fn count_citations(value: &Value) -> (usize, usize) {
    match value {
        Value::Object(obj) if obj.contains_key("value") && obj.contains_key("confidence") => {
            let cited = obj.get("page_number").map(|p| !p.is_null()).unwrap_or(false);
            (if cited { 1 } else { 0 }, 1)
        }
        Value::Object(obj) => obj.values().fold((0, 0), |(c, s), v| {
            let (c2, s2) = count_citations(v);
            (c + c2, s + s2)
        }),
        Value::Array(arr) => arr.iter().fold((0, 0), |(c, s), v| {
            let (c2, s2) = count_citations(v);
            (c + c2, s + s2)
        }),
        _ => (0, 0),
    }
}

fn annotate(
    value: &Value,
    lines: &[DiLine],
    generated_text: &str,
    logprobs: &[Value],
    confidences: &mut Vec<f64>,
) -> Value {
    match value {
        Value::Object(obj) => {
            let mut out = serde_json::Map::with_capacity(obj.len());
            for (k, v) in obj {
                out.insert(k.clone(), annotate(v, lines, generated_text, logprobs, confidences));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| annotate(v, lines, generated_text, logprobs, confidences))
                .collect(),
        ),
        Value::Null => value.clone(),
        scalar => {
            let (cu_conf, citation) = cu_field_confidence(scalar, lines);
            let gpt_conf = gpt_field_confidence(scalar, generated_text, logprobs);
            let merged = merge(cu_conf, gpt_conf);
            confidences.push(merged);
            serde_json::json!({
                "value": scalar,
                "confidence": merged,
                "page_number": citation.as_ref().and_then(|c| c.page_number),
                "text_offset_start": citation.as_ref().and_then(|c| c.text_offset_start),
                "text_offset_end": citation.as_ref().and_then(|c| c.text_offset_end),
            })
        }
    }
}
