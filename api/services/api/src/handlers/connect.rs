//! GIA -- the SageSure AI assistant. Ported from sagesure-us's
//! `handlers/gateway.rs::connect_chat`/`connect_memory`/`connect_history`,
//! which sagepas had no equivalent of at all (confirmed by grep: zero
//! references anywhere, backend or frontend, before this).
//!
//! Conversation memory (chat history + regex-extracted facts) plus, as of
//! work order §11, live per-record context via `context_providers` -- the
//! "does not query live platform data" gap this module's comment used to
//! name is now closed for the surfaces that have a provider (FNOL, UW).
//! Every fact a provider supplies is traceable to the record it read
//! (§11.2); authorization for *that record* is decided inside the provider
//! against the caller's own validated role, never assumed from the
//! request having reached this handler at all (§11.3).
//!
//! Adapted for sagepas's own clients: `state.openai_client` (the ported
//! `OpenAIClient`, Responses-API shaped) instead of manually building an
//! Azure OpenAI URL, and `state.search` for KB retrieval instead of a
//! hand-rolled search call.
//!
//! `/chat` is auth-required, not anonymous (work order §12.2, held as of
//! 2026-08-25): an anonymous LLM-backed endpoint needs per-IP rate
//! limiting, a per-session token ceiling, and a spend cap demonstrated
//! failing closed, none of which exist yet -- this operation has a real
//! prior $38k/4B-token runaway-inference incident, so this is a cost
//! decision, not a security one. `/memory` and `/history` stay
//! auth-required regardless (they're inherently identity-scoped: "my saved
//! facts" has no anonymous meaning).

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use conversation_memory::extract_facts_from_turn;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::auth_extract::AuthUser;
use crate::context_providers;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub tab: String,
    #[serde(default)]
    pub workflow_id: Option<String>,
    /// The record currently open on `tab` (e.g. an FNOL process_id, a UW
    /// job_id) -- work order §11.1. Absent means no page-scoped context;
    /// GIA still answers from KB + conversation memory as before.
    #[serde(default)]
    pub record_id: Option<String>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub reply: String,
}

#[derive(Deserialize)]
pub struct ConnectHistoryQuery {
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

async fn extract_optional_user_id(headers: &HeaderMap, state: &AppState) -> Option<Uuid> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))?;
    // sagepas's AuthenticatedUser carries `oid: String` (Entra object id, or
    // the dev-local user's UUID string) rather than sagesure-us's `sub:
    // Uuid` -- both are GUID-shaped, so parse rather than assume a type match.
    // Try staff then consumer (Phase 8) -- same order as middleware::require_auth.
    if let Some(validator) = &state.entra_staff_validator {
        if let Ok(user) = validator.validate(token).await {
            return Uuid::parse_str(&user.oid).ok();
        }
    }
    let validator = state.entra_consumer_validator.as_ref()?;
    let user = validator.validate(token).await.ok()?;
    Uuid::parse_str(&user.oid).ok()
}

fn normalize_role(role: &str) -> &str {
    match role {
        "assistant" => "assistant",
        "system" => "system",
        _ => "user",
    }
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "missing or invalid bearer token" })),
    )
        .into_response()
}

/// POST /api/v1/connect/chat
///
/// Still auth-required (work order §12.2, held: this path does not open to
/// anonymous callers until per-IP rate limiting, a per-session token
/// ceiling, and a spend cap demonstrated failing closed all exist -- this
/// operation has a real prior $38k/4B-token runaway-inference incident).
/// `AuthUser` is therefore a required extractor here, not optional.
pub async fn chat(
    State(state): State<AppState>,
    AuthUser(auth_user): AuthUser,
    headers: HeaderMap,
    Json(body): Json<ChatRequest>,
) -> Json<ChatResponse> {
    if state.config.azure_openai_key.is_empty() {
        return Json(ChatResponse {
            reply: "AI not configured.".to_string(),
        });
    }

    let user_id = Uuid::parse_str(&auth_user.oid).ok();
    let surface = headers
        .get("X-Surface")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .or_else(|| {
            if body.tab.trim().is_empty() {
                None
            } else {
                Some(body.tab.trim().to_string())
            }
        })
        .unwrap_or_else(|| "chat".to_string());

    let kb_context = match state
        .search
        .search_kb("insurance-general-kb", &body.message, None, 3)
        .await
    {
        Ok(results) if !results.is_empty() => {
            let docs: Vec<String> = results
                .iter()
                .map(|r| {
                    let snippet = &r.content[..r.content.len().min(600)];
                    format!(
                        "[{}] {}\n{}",
                        r.category.as_deref().unwrap_or(""),
                        r.title.as_deref().unwrap_or(""),
                        snippet
                    )
                })
                .collect();
            format!("\n\n---\nKnowledge Base:\n{}\n---", docs.join("\n\n"))
        }
        _ => String::new(),
    };

    let mut system = if body.system_prompt.is_empty() {
        format!(
            "You are SageGIA, SageSure AI assistant. Tab: {}. Be concise and helpful.{}",
            if body.tab.is_empty() { "dashboard" } else { &body.tab },
            kb_context
        )
    } else {
        format!("{}{}", body.system_prompt, kb_context)
    };

    // Work order §11.1/§11.3: record context is per-surface and
    // authorization-gated in the provider itself, not implied here by "a
    // record_id was supplied." No provider for this surface, no record_id,
    // record not found, or the caller's role isn't allowed -- all result in
    // no context attached, silently, never an error that breaks the chat.
    if let Some(record_id) = body.record_id.as_deref() {
        if let Some(provider) = context_providers::provider_for(&surface) {
            match provider.describe(&state, &auth_user, record_id).await {
                Ok(ctx) => system = format!("{}{}", system, ctx.to_prompt_block()),
                Err(context_providers::ContextError::Forbidden) => {
                    tracing::info!(%surface, %record_id, oid = %auth_user.oid, "GIA context denied: caller role not permitted for this surface");
                }
                Err(context_providers::ContextError::NotFound) => {
                    tracing::warn!(%surface, %record_id, "GIA context: record not found");
                }
                Err(context_providers::ContextError::Internal(e)) => {
                    tracing::error!(%surface, %record_id, error = %e, "GIA context provider failed");
                }
            }
        }
    }

    let mut memory_messages: Option<Vec<serde_json::Value>> = None;
    let mut memory_session_id: Option<Uuid> = None;

    if let Some(uid) = user_id {
        match state
            .conversation
            .get_or_create_session(uid, &surface, body.workflow_id.as_deref())
            .await
        {
            Ok(session) => {
                memory_session_id = Some(session.session_id);
                match state
                    .conversation
                    .build_context(uid, session.session_id, &body.message)
                    .await
                {
                    Ok(context) => {
                        if !context.system_prompt_injection.is_empty() {
                            system = format!("{}\n\n{}", context.system_prompt_injection, system);
                        }
                        memory_messages = Some(
                            context
                                .messages
                                .iter()
                                .map(|m| {
                                    serde_json::json!({
                                        "role": normalize_role(&m.role),
                                        "content": m.content,
                                    })
                                })
                                .collect(),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("conversation context build failed, stateless fallback: {}", e);
                        memory_session_id = None;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("conversation session init failed, stateless fallback: {}", e);
            }
        }
    }

    let mut messages = vec![serde_json::json!({ "role": "system", "content": system })];
    if let Some(mem_history) = memory_messages {
        messages.extend(mem_history);
    } else {
        for h in &body.history {
            messages.push(serde_json::json!({ "role": h.role, "content": h.content }));
        }
    }
    messages.push(serde_json::json!({ "role": "user", "content": body.message }));

    let reply = match state.openai_client.chat_completion(&messages, "", None, Some(800)).await {
        Ok(data) => data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("No response")
            .to_string(),
        Err(e) => {
            tracing::error!("AOAI failed: {}", e);
            "Could not reach AI service.".to_string()
        }
    };

    if let (Some(uid), Some(session_id)) = (user_id, memory_session_id) {
        if let Err(e) = state.conversation.append_message(session_id, "user", &body.message, &surface).await {
            tracing::warn!("append user message failed: {}", e);
        }
        if let Err(e) = state.conversation.append_message(session_id, "assistant", &reply, &surface).await {
            tracing::warn!("append assistant message failed: {}", e);
        }
        for fact in extract_facts_from_turn(&body.message, &reply) {
            if let Err(e) = state
                .conversation
                .upsert_fact(uid, session_id, &fact.fact_type, &fact.fact_key, &fact.fact_value, &surface)
                .await
            {
                tracing::warn!("fact upsert failed: {}", e);
            }
        }
    }

    Json(ChatResponse { reply })
}

/// GET /api/v1/connect/memory
pub async fn memory(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(user_id) = extract_optional_user_id(&headers, &state).await else {
        return unauthorized_response();
    };

    let facts = match state.conversation.load_facts(user_id).await {
        Ok(facts) => facts,
        Err(e) => {
            tracing::error!("connect memory load facts failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to load memory" }))).into_response();
        }
    };

    let active_session = match sqlx::query(
        r#"
        SELECT session_id, surface, workflow_id, last_active_at
        FROM conversation_sessions
        WHERE user_id = $1
        ORDER BY last_active_at DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_optional(&**state.db)
    .await
    {
        Ok(row_opt) => row_opt.map(|row| {
            serde_json::json!({
                "session_id": row.try_get::<Uuid, _>("session_id").ok(),
                "surface": row.try_get::<String, _>("surface").ok(),
                "workflow_id": row.try_get::<Option<String>, _>("workflow_id").ok().flatten(),
                "last_active_at": row.try_get::<DateTime<Utc>, _>("last_active_at").ok(),
            })
        }),
        Err(e) => {
            tracing::warn!("connect memory active session lookup failed: {}", e);
            None
        }
    };

    let latest_summary = if let Some(active) = &active_session {
        if let Some(sid) = active.get("session_id").and_then(|v| serde_json::from_value::<Uuid>(v.clone()).ok()) {
            sqlx::query(
                r#"
                SELECT summary_text, message_count, created_at
                FROM conversation_summaries
                WHERE session_id = $1
                ORDER BY created_at DESC
                LIMIT 1
                "#,
            )
            .bind(sid)
            .fetch_optional(&**state.db)
            .await
            .ok()
            .flatten()
            .map(|row| {
                serde_json::json!({
                    "summary_text": row.try_get::<String, _>("summary_text").ok(),
                    "message_count": row.try_get::<i32, _>("message_count").ok(),
                    "created_at": row.try_get::<DateTime<Utc>, _>("created_at").ok(),
                })
            })
        } else {
            None
        }
    } else {
        None
    };

    Json(serde_json::json!({
        "user_id": user_id,
        "facts": facts,
        "active_session": active_session,
        "latest_summary": latest_summary,
    }))
    .into_response()
}

/// GET /api/v1/connect/history
pub async fn history(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<ConnectHistoryQuery>) -> Response {
    let Some(user_id) = extract_optional_user_id(&headers, &state).await else {
        return unauthorized_response();
    };

    let session_id = if let Some(session_id) = q.session_id {
        session_id
    } else {
        match sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT session_id FROM conversation_sessions
            WHERE user_id = $1 ORDER BY last_active_at DESC LIMIT 1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&**state.db)
        .await
        {
            Ok(Some(id)) => id,
            Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "no session found" }))).into_response(),
            Err(e) => {
                tracing::error!("connect history session lookup failed: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to load history" }))).into_response();
            }
        }
    };

    let owns_session = match sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(SELECT 1 FROM conversation_sessions WHERE session_id = $1 AND user_id = $2)"#,
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_one(&**state.db)
    .await
    {
        Ok(flag) => flag,
        Err(e) => {
            tracing::error!("connect history ownership check failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to load history" }))).into_response();
        }
    };

    if !owns_session {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "session not found" }))).into_response();
    }

    let messages = match state.conversation.load_recent_messages(session_id, 200).await {
        Ok(messages) => messages,
        Err(e) => {
            tracing::error!("connect history messages load failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to load history" }))).into_response();
        }
    };

    Json(serde_json::json!({
        "session_id": session_id,
        "messages": messages,
    }))
    .into_response()
}
