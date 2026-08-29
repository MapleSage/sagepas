//! Native WhatsApp send/webhook, ported from sesure-us's `gateway.rs`
//! (`whatsapp_send_native`, `whatsapp_webhook_verify`, `whatsapp_webhook_receive`,
//! `whatsapp_send_flow_native`) onto sagepas's `AuthUser`/`AppState`.
//!
//! Unlike sesure-us, there is no separate Python `notifications` service
//! behind this -- the `whatsapp` crate is the only send path, and there is
//! no templated `/notify/send` endpoint to proxy (sesure-us's own
//! `notification_type` -> template lookup lives only in that Python
//! service and was deliberately not ported, to avoid silently wrong
//! message text -- see the `whatsapp` crate's doc comment).
//!
//! Auth split, per work order Phase 7: the inbound webhook stays
//! unauthenticated (Meta calls it directly, no bearer token available) and
//! is instead verified via the `X-Hub-Signature-256` HMAC header. The send
//! endpoints require an authenticated staff caller -- they are triggered by
//! server-side UI actions (claim/policy status changes), not raw end-user
//! input.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth_extract::{require_roles, AuthUser};
use crate::state::AppState;
use domain::auth::PasRole;

const STAFF_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];

/// The SageSure claims-intake flow, published in the WhatsApp Business
/// Manager under the SageSure AI number. `CLAIM_TYPE` is its first screen.
const SAGESURE_CLAIM_FLOW_ID: &str = "1651698452817963";
const SAGESURE_CLAIM_FLOW_FIRST_SCREEN: &str = "CLAIM_TYPE";

fn not_configured(name: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": format!("{name} is not configured") })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct WhatsAppSendRequest {
    pub phone: String,
    pub message: String,
}

/// POST /api/v1/notify/whatsapp/send -- staff/system-authenticated.
pub async fn whatsapp_send(
    State(s): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<WhatsAppSendRequest>,
) -> Response {
    if let Err(e) = require_roles(&user, STAFF_ROLES) {
        return e.into_response();
    }
    let Some(client) = s.whatsapp_client.clone() else {
        return not_configured("whatsapp");
    };

    match client.send_text(&req.phone, &req.message).await {
        Ok(()) => Json(json!({ "status": "sent" })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, phone = %req.phone, "native WhatsApp send failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "status": "failed", "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

fn default_flow_cta() -> String {
    "File a Claim".to_string()
}

fn default_flow_body() -> String {
    "Tap below to file your claim with SageSure — it only takes a couple of minutes.".to_string()
}

#[derive(Deserialize)]
pub struct WhatsAppSendFlowRequest {
    pub phone: String,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub first_screen: Option<String>,
    #[serde(default = "default_flow_cta")]
    pub flow_cta: String,
    #[serde(default = "default_flow_body")]
    pub body_text: String,
}

/// POST /api/v1/notify/whatsapp/send-flow -- staff/system-authenticated.
pub async fn whatsapp_send_flow(
    State(s): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<WhatsAppSendFlowRequest>,
) -> Response {
    if let Err(e) = require_roles(&user, STAFF_ROLES) {
        return e.into_response();
    }
    let Some(client) = s.whatsapp_client.clone() else {
        return not_configured("whatsapp");
    };

    let flow_id = req.flow_id.as_deref().unwrap_or(SAGESURE_CLAIM_FLOW_ID);
    let first_screen = req.first_screen.as_deref().unwrap_or(SAGESURE_CLAIM_FLOW_FIRST_SCREEN);

    match client
        .send_flow(&req.phone, flow_id, first_screen, &req.flow_cta, &req.body_text)
        .await
    {
        Ok(()) => Json(json!({ "status": "sent", "flow_id": flow_id })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, phone = %req.phone, flow_id, "native WhatsApp flow send failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "status": "failed", "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct WhatsAppWebhookVerifyQuery {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}

/// GET /api/v1/notify/whatsapp/webhook -- unauthenticated. One-time
/// verification handshake Meta performs when the webhook URL is first
/// registered in the App Dashboard.
pub async fn whatsapp_webhook_verify(
    State(s): State<AppState>,
    Query(q): Query<WhatsAppWebhookVerifyQuery>,
) -> Response {
    let configured = &s.config.whatsapp_webhook_verify_token;
    if configured.is_empty() {
        tracing::error!("WhatsApp webhook verify attempted but WHATSAPP_WEBHOOK_VERIFY_TOKEN is not set");
        return (StatusCode::FORBIDDEN, "webhook not configured").into_response();
    }

    match (q.mode.as_deref(), q.verify_token.as_deref(), q.challenge) {
        (Some("subscribe"), Some(token), Some(challenge)) if token == configured => {
            challenge.into_response()
        }
        _ => (StatusCode::FORBIDDEN, "verification failed").into_response(),
    }
}

fn verify_whatsapp_signature(app_secret: &str, signature_header: Option<&str>, raw_body: &[u8]) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    if app_secret.is_empty() {
        // No secret configured — accept unverified. Wiring-stage default,
        // not a production posture; see WhatsAppConfig doc comment.
        return true;
    }
    let Some(header) = signature_header else { return false };
    let Some(hex_sig) = header.strip_prefix("sha256=") else { return false };
    let Ok(expected_bytes) = hex::decode(hex_sig) else { return false };

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()) else { return false };
    mac.update(raw_body);
    mac.verify_slice(&expected_bytes).is_ok()
}

/// POST /api/v1/notify/whatsapp/webhook -- unauthenticated (verified via
/// HMAC signature instead). Inbound message replies, delivery statuses, and
/// Flow completions (`interactive.nfm_reply`). Always acks fast with 200
/// (Meta retries/backs off aggressively on non-2xx), even when the payload
/// shape is unrecognized — log and move on rather than fail a webhook Meta
/// will keep retrying.
pub async fn whatsapp_webhook_receive(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok());

    if !verify_whatsapp_signature(&s.config.whatsapp_app_secret, signature, &body) {
        tracing::warn!("WhatsApp webhook signature verification failed");
        return StatusCode::FORBIDDEN.into_response();
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "WhatsApp webhook payload was not valid JSON");
            return StatusCode::OK.into_response();
        }
    };

    let messages = payload["entry"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["changes"].as_array().cloned().unwrap_or_default())
        .flat_map(|change| {
            change["value"]["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        });

    for msg in messages {
        let from = msg["from"].as_str().unwrap_or("unknown");
        if let Some(reply) = msg["interactive"]["nfm_reply"].as_object() {
            let flow_token = reply
                .get("flow_token")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let response_json = reply.get("response_json").and_then(|v| v.as_str());
            tracing::info!(from, flow_token, response_json, "WhatsApp flow completion received");
        } else if let Some(text) = msg["text"]["body"].as_str() {
            tracing::info!(from, text, "WhatsApp message received");
            if let Some(client) = s.whatsapp_client.clone() {
                let from = from.to_string();
                match client
                    .send_flow(
                        &from,
                        SAGESURE_CLAIM_FLOW_ID,
                        SAGESURE_CLAIM_FLOW_FIRST_SCREEN,
                        &default_flow_cta(),
                        &default_flow_body(),
                    )
                    .await
                {
                    Ok(()) => tracing::info!(from, "WhatsApp claim flow sent in reply to inbound message"),
                    Err(e) => tracing::error!(error = %e, from, "WhatsApp claim flow reply failed"),
                }
            } else {
                tracing::warn!(from, "WhatsApp client not configured, cannot reply");
            }
        } else {
            tracing::info!(from, msg = %msg, "WhatsApp event received (unhandled shape)");
        }
    }

    StatusCode::OK.into_response()
}
