//! FNOL intake -- one of the two callers of the shared `doc-pipeline` engine
//! (consolidation decision, `CLAUDE.md` 2026-08-23). Never forks the
//! pipeline; supplies its own `PipelineConfig` (output: HubSpot ticket) and
//! owns HubSpot ticket creation + the human-in-loop routing decision, both
//! deliberately domain-specific and kept out of `doc-pipeline` itself.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use doc_pipeline::{Domain, OutputKind, PipelineConfig};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

fn bad_request(msg: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.into())
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(Debug, Serialize)]
pub struct FnolSubmitResponse {
    pub process_id: Uuid,
    pub ticket_id: Option<String>,
    pub status: String,
    pub confidence: f64,
    pub human_review_required: bool,
    pub summary: serde_json::Value,
}

/// POST /api/v1/fnol/submit -- multipart: `file` (required), `email`,
/// `name`, `phone` (required -- identity resolution happens before anything
/// else, same as the prospect-quote and customer-creation paths), and
/// `insurance_type` (schema key: auto/life/health/property/marine).
pub async fn submit(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<FnolSubmitResponse>, (StatusCode, String)> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut original_filename = String::new();
    let mut mime_type = "application/octet-stream".to_string();
    let mut email = String::new();
    let mut name = String::new();
    let mut phone = String::new();
    let mut insurance_type = String::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| bad_request(e.to_string()))? {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "file" => {
                original_filename = field.file_name().unwrap_or("upload").to_string();
                mime_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let data = field.bytes().await.map_err(|e| bad_request(e.to_string()))?;
                file_bytes = Some(data.to_vec());
            }
            "email" => email = field.text().await.unwrap_or_default().trim().to_lowercase(),
            "name" => name = field.text().await.unwrap_or_default().trim().to_string(),
            "phone" => phone = field.text().await.unwrap_or_default().trim().to_string(),
            "insurance_type" => {
                insurance_type = field.text().await.unwrap_or_default().trim().to_lowercase()
            }
            _ => {}
        }
    }

    let file_bytes = file_bytes.ok_or_else(|| bad_request("multipart field 'file' is required"))?;
    if email.is_empty() {
        return Err(bad_request("multipart field 'email' is required"));
    }
    if file_bytes.is_empty() {
        return Err(bad_request("uploaded file is empty"));
    }

    // Identity resolution before anything else -- same discipline as every
    // other intake path since item 1: no record can exist without a
    // counterpart in the CRM.
    let contact_object_id = crate::hubspot_bridge::find_or_create_contact(&state, &email, &name, &phone).await?;

    let process_id = Uuid::new_v4();
    let blob_name = format!("{process_id}/{original_filename}");
    state
        .fnol_blob
        .upload(&blob_name, file_bytes.clone(), &mime_type)
        .await
        .map_err(internal)?;

    sqlx::query(
        r#"
        INSERT INTO fnol_submissions (process_id, blob_container, blob_name, original_filename, mime_type, status, human_review_required)
        VALUES ($1, 'fnol-documents', $2, $3, $4, 'processing', true)
        "#,
    )
    .bind(process_id)
    .bind(&blob_name)
    .bind(&original_filename)
    .bind(&mime_type)
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    // Ingest event -- owned here, at the point of ingest, not tacked onto
    // the Save-stage write below. `occurred_at` is the domain event's own
    // timestamp; conflating it with when the pipeline happened to finish
    // writing is the ownership bug that produced the NOT NULL violation
    // this insert used to hit (see CLAUDE.md's failure log discipline: fix
    // the ownership, not just the column).
    sqlx::query(
        r#"
        INSERT INTO fnol_events (event_type, process_id, occurred_at, payload)
        VALUES ('fnol.submitted', $1, $2, $3)
        "#,
    )
    .bind(process_id)
    .bind(chrono::Utc::now())
    .bind(serde_json::json!({ "original_filename": original_filename, "mime_type": mime_type }))
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    let config = PipelineConfig {
        domain: Domain::Fnol,
        field_schema_key: insurance_type.clone(),
        kb_index: format!("insurance-{}-kb", if insurance_type.is_empty() { "general" } else { &insurance_type }),
        output_kind: OutputKind::HubspotTicket,
    };

    let result = doc_pipeline::run_pipeline(
        &file_bytes,
        &mime_type,
        state.cu_client.as_deref(),
        &state.openai_client,
        &config,
    )
    .await
    .map_err(internal)?;

    // KB-grounded severity assessment. Not a black box: every conclusion
    // GPT draws is required to cite which retrieved KB passage supports it,
    // and the caller (this function) attaches the full passage text, not a
    // summary -- see kb_scoring.rs for why this is stricter than either
    // reference implementation.
    let kb_query = format!(
        "{} claim severity assessment {}",
        if insurance_type.is_empty() { "insurance" } else { &insurance_type },
        result.extracted_json,
    );
    let kb_result = doc_pipeline::kb_scoring::score_with_kb(
        &state.search,
        &state.openai_client,
        &config.kb_index,
        &kb_query,
        "You are assessing the severity and completeness of a first-notice-of-loss insurance claim. \
         Draw conclusions about severity indicators, missing information, and fraud-risk signals, \
         grounded strictly in the retrieved claims-handling guidance below.",
        &result.extracted_json,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, %process_id, "KB-grounded scoring failed, proceeding without it");
        doc_pipeline::kb_scoring::KbScoringResult {
            conclusions: Vec::new(),
            retrieved_count: 0,
            uncited_count: 0,
        }
    });

    // Ticket creation happens regardless of confidence -- a low-confidence
    // submission still needs a place for a human to review it; that's what
    // human_review_required + the ticket queue are for, not a reason to
    // withhold the ticket.
    let subject = format!("FNOL: {} ({})", original_filename, if insurance_type.is_empty() { "unspecified" } else { &insurance_type });
    let content = format!(
        "Automated FNOL intake. Confidence: {:.0}%. Review required: {}.\n\nExtracted summary:\n{}\n\nKB-grounded findings ({} of {} retrieved passages cited):\n{}",
        result.confidence * 100.0,
        result.human_review_required,
        serde_json::to_string_pretty(&result.summary_json).unwrap_or_default(),
        kb_result.retrieved_count - kb_result.uncited_count,
        kb_result.retrieved_count,
        kb_result.conclusions.iter().map(|c| format!("- {}", c.statement)).collect::<Vec<_>>().join("\n"),
    );
    let ticket_id = crate::hubspot_bridge::find_or_create_ticket(&state, &contact_object_id, &subject, &content)
        .await
        .map(Some)
        .unwrap_or_else(|(_, err)| {
            tracing::error!(error = %err, %process_id, "HubSpot ticket creation failed, FNOL submission persisted without one");
            None
        });

    let status = if result.human_review_required { "review_required" } else { "processed" };

    // kb_findings folded into summary_json rather than a new column -- this
    // IS the scoring-stage summary, not a separate concept. Full citation
    // text travels with it (kb_result.conclusions[].citations[].passage_text
    // is the untruncated Azure Search content), so GET .../trace can trace
    // a conclusion back to its source passage without a second query.
    let mut summary_with_kb = result.summary_json.clone();
    if let Some(obj) = summary_with_kb.as_object_mut() {
        obj.insert("kb_findings".to_string(), serde_json::to_value(&kb_result).unwrap_or_default());
    }

    sqlx::query(
        r#"
        UPDATE fnol_submissions
        SET ticket_id = $2, status = $3, extracted_json = $4, summary_json = $5,
            confidence = $6, human_review_required = $7, stages_json = $8, updated_at = now()
        WHERE process_id = $1
        "#,
    )
    .bind(process_id)
    .bind(&ticket_id)
    .bind(status)
    .bind(&result.extracted_json)
    .bind(&summary_with_kb)
    .bind(result.confidence)
    .bind(result.human_review_required)
    .bind(serde_json::to_value(&result.stages).unwrap_or_default())
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    // Save-stage event, distinct from Ingest's 'fnol.submitted' above --
    // its own name, its own occurred_at, not borrowing Ingest's.
    sqlx::query(
        r#"
        INSERT INTO fnol_events (event_type, process_id, occurred_at, payload)
        VALUES ('fnol.completed', $1, $2, $3)
        "#,
    )
    .bind(process_id)
    .bind(chrono::Utc::now())
    .bind(serde_json::json!({ "ticket_id": ticket_id, "confidence": result.confidence }))
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(FnolSubmitResponse {
        process_id,
        ticket_id,
        status: status.to_string(),
        confidence: result.confidence,
        human_review_required: result.human_review_required,
        summary: result.summary_json,
    }))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FnolListRow {
    pub process_id: Uuid,
    pub ticket_id: Option<String>,
    pub original_filename: Option<String>,
    pub status: String,
    pub confidence: Option<f64>,
    pub human_review_required: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/v1/fnol/submissions -- the intake queue.
pub async fn list_submissions(State(state): State<AppState>) -> Result<Json<Vec<FnolListRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, FnolListRow>(
        r#"
        SELECT process_id, ticket_id, original_filename, status, confidence, human_review_required, created_at
        FROM fnol_submissions ORDER BY created_at DESC LIMIT 200
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FnolTraceRow {
    pub process_id: Uuid,
    pub ticket_id: Option<String>,
    pub status: String,
    pub extracted_json: Option<serde_json::Value>,
    pub summary_json: Option<serde_json::Value>,
    pub confidence: Option<f64>,
    pub human_review_required: bool,
    pub stages_json: Option<serde_json::Value>,
}

/// GET /api/v1/fnol/:id/trace -- the stage-by-stage read endpoint the
/// consolidation decision's acceptance test opens: every stage individually
/// inspectable via `stages_json`, not just the final extracted/summary blob.
pub async fn trace(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FnolTraceRow>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, FnolTraceRow>(
        r#"
        SELECT process_id, ticket_id, status, extracted_json, summary_json, confidence, human_review_required, stages_json
        FROM fnol_submissions WHERE process_id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "fnol submission not found".to_string()))?;

    Ok(Json(row))
}
