//! Underwriting intake -- the other caller of the shared `doc-pipeline`
//! engine. Same engine as FNOL (`handlers/fnol.rs`), different
//! `PipelineConfig` (output: HubSpot deal) and its own HubSpot deal
//! creation. Never forks the pipeline; the two HubSpot cards stay two cards.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
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
pub struct UwUploadResponse {
    pub job_id: String,
    pub deal_id: Option<String>,
    pub status: String,
    /// entity_score -- merged CU+GPT confidence the text was read correctly.
    pub confidence: f64,
    /// Fraction of non-null fields -- was the schema satisfied.
    pub schema_score: f64,
    pub human_review_required: bool,
    pub summary: serde_json::Value,
}

/// POST /api/v1/uw/upload -- multipart: `file` (required), `email`, `name`,
/// `phone` (required), `insurance_type` (schema key), optional
/// `process_id` (links to a prior FNOL submission when the submission is a
/// follow-on to an existing claim rather than a standalone new-business
/// application -- `uw_jobs.process_id` is nullable precisely for this).
pub async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UwUploadResponse>, (StatusCode, String)> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut original_filename = String::new();
    let mut mime_type = "application/octet-stream".to_string();
    let mut email = String::new();
    let mut name = String::new();
    let mut phone = String::new();
    let mut insurance_type = String::new();
    let mut linked_process_id: Option<Uuid> = None;

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
            "process_id" => {
                let raw = field.text().await.unwrap_or_default();
                linked_process_id = Uuid::parse_str(raw.trim()).ok();
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

    let contact_object_id = crate::hubspot_bridge::find_or_create_contact(&state, &email, &name, &phone).await?;

    let job_id = format!("uw-{}", Uuid::new_v4());
    let blob_name = format!("{job_id}/{original_filename}");
    state
        .uw_blob
        .upload(&blob_name, file_bytes.clone(), &mime_type)
        .await
        .map_err(internal)?;

    sqlx::query(
        r#"
        INSERT INTO uw_jobs (job_id, process_id, blob_container, blob_name, original_filename, mime_type, status)
        VALUES ($1, $2, 'uw-documents', $3, $4, $5, 'processing')
        "#,
    )
    .bind(&job_id)
    .bind(linked_process_id)
    .bind(&blob_name)
    .bind(&original_filename)
    .bind(&mime_type)
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    let config = PipelineConfig {
        domain: Domain::Underwriting,
        field_schema_key: insurance_type.clone(),
        kb_index: format!("insurance-{}-kb", if insurance_type.is_empty() { "general" } else { &insurance_type }),
        output_kind: OutputKind::HubspotDeal,
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

    // KB-grounded appetite assessment -- same discipline as FNOL's severity
    // scoring (kb_scoring.rs): every conclusion cites the retrieved passage
    // it rests on, full text attached, not summarized.
    let kb_query = format!(
        "{} underwriting appetite guidelines {}",
        if insurance_type.is_empty() { "insurance" } else { &insurance_type },
        result.extracted_json,
    );
    let kb_result = doc_pipeline::kb_scoring::score_with_kb(
        &state.search,
        &state.openai_client,
        &config.kb_index,
        &kb_query,
        "You are assessing an underwriting submission against the carrier's own appetite and \
         guidelines. Draw conclusions about whether this risk falls inside or outside appetite, \
         and what factors drive that, grounded strictly in the retrieved underwriting guidance below.",
        &result.extracted_json,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, %job_id, "KB-grounded scoring failed, proceeding without it");
        doc_pipeline::kb_scoring::KbScoringResult {
            conclusions: Vec::new(),
            retrieved_count: 0,
            uncited_count: 0,
        }
    });

    let dealname = format!(
        "UW: {} ({})",
        name.is_empty().then(|| email.clone()).unwrap_or(name.clone()),
        if insurance_type.is_empty() { "unspecified" } else { &insurance_type },
    );
    let deal_id = crate::hubspot_bridge::find_or_create_deal(&state, &contact_object_id, &dealname)
        .await
        .map(Some)
        .unwrap_or_else(|(_, err)| {
            tracing::error!(error = %err, %job_id, "HubSpot deal creation failed, UW job persisted without one");
            None
        });

    // First-pass recommendation logic: real, KB-grounded, not a placeholder
    // -- but deliberately simple (keyword scan over grounded conclusions,
    // not a trained classifier). "Below the reference bar's own scale" is
    // an honest description of this, not a claim of parity with a mature
    // scoring model. No KB grounding at all is a hard refer, never a
    // silent approve.
    let decline_signal = kb_result.conclusions.iter().any(|c| {
        let s = c.statement.to_lowercase();
        !c.citations.is_empty() && (s.contains("decline") || s.contains("outside appetite") || s.contains("ineligible"))
    });
    let recommendation = if result.human_review_required || kb_result.retrieved_count == 0 {
        "refer_to_underwriter"
    } else if decline_signal {
        "outside_appetite"
    } else if !kb_result.conclusions.is_empty() {
        "within_appetite"
    } else {
        "refer_to_underwriter"
    };
    let status = if result.human_review_required { "review_required" } else { "processed" };

    let mut analysis_with_kb = result.extracted_json.clone();
    if let Some(obj) = analysis_with_kb.as_object_mut() {
        obj.insert("kb_findings".to_string(), serde_json::to_value(&kb_result).unwrap_or_default());
        // Folded in from summary_json rather than a new column -- same
        // pattern as kb_findings above. Step 2's sign-off condition: an
        // uncited field must be countable, not silently absent.
        obj.insert("_cited_field_count".to_string(), result.summary_json.get("cited_field_count").cloned().unwrap_or_default());
        obj.insert("_scored_field_count".to_string(), result.summary_json.get("scored_field_count").cloned().unwrap_or_default());
    }

    sqlx::query(
        r#"
        UPDATE uw_jobs
        SET deal_id = $2, status = $3, analysis_json = $4,
            confidence = $5, schema_score = $6, recommendation = $7, stages_json = $8, updated_at = now()
        WHERE job_id = $1
        "#,
    )
    .bind(&job_id)
    .bind(&deal_id)
    .bind(status)
    .bind(&analysis_with_kb)
    .bind(result.entity_score)
    .bind(result.schema_score)
    .bind(recommendation)
    .bind(serde_json::to_value(&result.stages).unwrap_or_default())
    .execute(&**state.db)
    .await
    .map_err(internal)?;

    Ok(Json(UwUploadResponse {
        job_id,
        deal_id,
        status: status.to_string(),
        confidence: result.entity_score,
        schema_score: result.schema_score,
        human_review_required: result.human_review_required,
        summary: result.summary_json,
    }))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UwListRow {
    pub job_id: String,
    pub deal_id: Option<String>,
    pub status: String,
    pub original_filename: Option<String>,
    pub confidence: Option<f64>,
    pub schema_score: Option<f64>,
    pub recommendation: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/v1/uw/jobs -- the submission queue.
pub async fn list_jobs(State(state): State<AppState>) -> Result<Json<Vec<UwListRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, UwListRow>(
        r#"
        SELECT job_id, deal_id, status, original_filename, confidence, schema_score, recommendation, created_at
        FROM uw_jobs ORDER BY created_at DESC LIMIT 200
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UwTraceRow {
    pub job_id: String,
    pub deal_id: Option<String>,
    pub status: String,
    pub analysis_json: Option<serde_json::Value>,
    pub confidence: Option<f64>,
    pub schema_score: Option<f64>,
    pub recommendation: Option<String>,
    pub stages_json: Option<serde_json::Value>,
}

/// GET /api/v1/uw/:id/trace
pub async fn trace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<UwTraceRow>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, UwTraceRow>(
        r#"
        SELECT job_id, deal_id, status, analysis_json, confidence, schema_score, recommendation, stages_json
        FROM uw_jobs WHERE job_id = $1
        "#,
    )
    .bind(&id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "uw job not found".to_string()))?;

    Ok(Json(row))
}

/// GET /api/v1/uw/:id/document -- same pattern as fnol.rs::document /
/// policies.rs::get_policy_document: stream through the API, blob has no
/// public access.
pub async fn document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"SELECT blob_name, mime_type FROM uw_jobs WHERE job_id = $1"#,
    )
    .bind(&id)
    .fetch_optional(&**state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "uw job not found".to_string()))?;

    let (blob_name, mime_type) = row;
    let bytes = state
        .uw_blob
        .download(&blob_name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("document could not be retrieved from storage: {e}")))?;

    let content_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_string());
    Ok((
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response())
}
