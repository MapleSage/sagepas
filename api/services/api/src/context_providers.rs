//! GIA's context-provider contract (work order §11.1): one interface, N
//! implementations. Every surface exposes a uniform descriptor of the
//! record currently open; GIA consumes the contract, never the page's own
//! shape. Adding a surface means writing a new `ContextProvider`, not
//! editing `connect::chat`.
//!
//! Two non-negotiable properties, both from §11.2/§11.3:
//!
//! - Every fact a provider states must be traceable to the record it read
//!   -- `SurfaceContext::citations` is that trail. `describe()` never
//!   returns free text GIA can't attribute back to a row.
//! - Authorization is the caller's, never the service's. `describe()` takes
//!   the already-validated `AuthenticatedUser` (the same one `require_auth`
//!   middleware attaches to every request) and is the one place that
//!   decides whether *this* caller may see *this* record -- retrofitting
//!   that check after GIA can already answer questions about a record is
//!   the expensive way to do it, per §11.3.

use domain::auth::{AuthenticatedUser, PasRole};
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug)]
pub enum ContextError {
    NotFound,
    Forbidden,
    Internal(String),
}

pub struct ContextCitation {
    /// e.g. "extracted_json.policyholder.first_name" or "stage:evaluate"
    pub field_path: String,
    pub value: String,
}

pub struct SurfaceContext {
    /// Injected into GIA's system prompt verbatim. Written so the model can
    /// answer "what do you see on this page" directly, and told to cite.
    pub summary: String,
    pub citations: Vec<ContextCitation>,
}

impl SurfaceContext {
    /// Renders as the system-prompt block `connect::chat` appends, plus the
    /// citation instruction -- §11.2's "resolves to the record it came
    /// from, or it is not rendered" applies to GIA's own output, not just
    /// the record data, so the instruction travels with the context.
    pub fn to_prompt_block(&self) -> String {
        let cites: String = self
            .citations
            .iter()
            .map(|c| format!("- {}: {}", c.field_path, c.value))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\n---\nOpen record context:\n{}\n\nSourced fields (cite these paths when you state a fact from them, e.g. \"(policy.face_amount)\"):\n{}\n\nIf a question can't be answered from this context or the knowledge base, say so -- do not guess at platform data.\n---",
            self.summary, cites
        )
    }
}

#[async_trait::async_trait]
pub trait ContextProvider: Send + Sync {
    fn surface(&self) -> &'static str;
    async fn describe(
        &self,
        state: &AppState,
        user: &AuthenticatedUser,
        record_id: &str,
    ) -> Result<SurfaceContext, ContextError>;
}

fn require_staff(user: &AuthenticatedUser) -> Result<(), ContextError> {
    // FNOL/UW traces are staff-facing queues today -- no customer-scoped
    // ownership column exists on fnol_submissions/uw_jobs to check a
    // customer against (there is no B2C route to either page yet either).
    // Staff-role gate is the real boundary until a customer-facing FNOL/UW
    // surface exists, at which point that surface needs its own row-level
    // ownership check, not a relaxed version of this one.
    if user.has_any_role(&[PasRole::Admin, PasRole::Agent, PasRole::Underwriter]) {
        Ok(())
    } else {
        Err(ContextError::Forbidden)
    }
}

pub struct FnolContextProvider;

#[async_trait::async_trait]
impl ContextProvider for FnolContextProvider {
    fn surface(&self) -> &'static str {
        "fnol"
    }

    async fn describe(
        &self,
        state: &AppState,
        user: &AuthenticatedUser,
        record_id: &str,
    ) -> Result<SurfaceContext, ContextError> {
        require_staff(user)?;
        let id = Uuid::parse_str(record_id).map_err(|_| ContextError::NotFound)?;

        let row = sqlx::query_as::<_, (Uuid, Option<String>, Option<String>, String, Option<serde_json::Value>, Option<f64>, Option<f64>, bool)>(
            r#"SELECT process_id, ticket_id, original_filename, status, extracted_json, confidence, schema_score, human_review_required
               FROM fnol_submissions WHERE process_id = $1"#,
        )
        .bind(id)
        .fetch_optional(&**state.db)
        .await
        .map_err(|e| ContextError::Internal(e.to_string()))?
        .ok_or(ContextError::NotFound)?;

        let (process_id, ticket_id, filename, status, extracted, confidence, schema_score, review) = row;

        let summary = format!(
            "FNOL submission {process_id} ({}). Status: {status}. Entity score: {}. Schema score: {}. Human review required: {review}. HubSpot ticket: {}.",
            filename.as_deref().unwrap_or("no filename"),
            confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "not yet scored".into()),
            schema_score.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "not yet scored".into()),
            ticket_id.as_deref().unwrap_or("none"),
        );

        let citations = extracted_json_to_citations(extracted.as_ref());

        Ok(SurfaceContext { summary, citations })
    }
}

pub struct UwContextProvider;

#[async_trait::async_trait]
impl ContextProvider for UwContextProvider {
    fn surface(&self) -> &'static str {
        "uw"
    }

    async fn describe(
        &self,
        state: &AppState,
        user: &AuthenticatedUser,
        record_id: &str,
    ) -> Result<SurfaceContext, ContextError> {
        require_staff(user)?;

        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>, String, Option<serde_json::Value>, Option<f64>, Option<f64>, Option<String>)>(
            r#"SELECT job_id, deal_id, original_filename, status, analysis_json, confidence, schema_score, recommendation
               FROM uw_jobs WHERE job_id = $1"#,
        )
        .bind(record_id)
        .fetch_optional(&**state.db)
        .await
        .map_err(|e| ContextError::Internal(e.to_string()))?
        .ok_or(ContextError::NotFound)?;

        let (job_id, deal_id, filename, status, analysis, confidence, schema_score, recommendation) = row;

        let summary = format!(
            "UW job {job_id} ({}). Status: {status}. Recommendation: {}. Entity score: {}. Schema score: {}. HubSpot deal: {}.",
            filename.as_deref().unwrap_or("no filename"),
            recommendation.as_deref().unwrap_or("not yet scored"),
            confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "not yet scored".into()),
            schema_score.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "not yet scored".into()),
            deal_id.as_deref().unwrap_or("none"),
        );

        let citations = extracted_json_to_citations(analysis.as_ref());

        Ok(SurfaceContext { summary, citations })
    }
}

/// Walks the `{value, confidence, page_number, ...}` citation-leaf shape
/// `doc_pipeline::evaluate` produces and turns each non-null leaf into a
/// `ContextCitation` GIA can quote -- same primitive `CitedFieldTree.tsx`
/// renders, reused here so GIA's citations and the UI's citations always
/// point at the same underlying data, not two independently-drifting copies.
fn extracted_json_to_citations(value: Option<&Value>) -> Vec<ContextCitation> {
    let mut out = Vec::new();
    if let Some(v) = value {
        walk(v, "", &mut out);
    }
    out
}

fn walk(value: &Value, path: &str, out: &mut Vec<ContextCitation>) {
    match value {
        Value::Object(obj) if obj.contains_key("value") && obj.contains_key("confidence") => {
            if let Some(v) = obj.get("value") {
                if !v.is_null() {
                    let rendered = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    out.push(ContextCitation {
                        field_path: path.trim_start_matches('.').to_string(),
                        value: rendered,
                    });
                }
            }
        }
        Value::Object(obj) => {
            for (k, v) in obj {
                if k == "kb_findings" || k.starts_with('_') {
                    continue;
                }
                walk(v, &format!("{path}.{k}"), out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                walk(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

pub fn provider_for(surface: &str) -> Option<Box<dyn ContextProvider>> {
    match surface {
        "fnol" => Some(Box::new(FnolContextProvider)),
        "uw" => Some(Box::new(UwContextProvider)),
        _ => None,
    }
}
