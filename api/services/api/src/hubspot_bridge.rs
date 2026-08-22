//! Shared HubSpot identity-resolution primitives (work order items 1 and 3).
//!
//! `find_or_create_contact` is the one place any PAS record creation path
//! resolves "which HubSpot contact is this email" through -- FNOL, UW, and
//! PAS itself all go through the equivalent of this call (PAS's own is here
//! since it's the same process; FNOL/UW live in sagesure-us and call the
//! same bridge operation directly). `ensure_link` builds on it: resolve,
//! then upsert the `hubspot_record_links` row in the same open transaction
//! as whatever customer/quote/policy write triggered it, so a customer
//! never ends up created without a link -- no separate manual step, no
//! human who has to remember to do it.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::state::AppState;

/// Matches sagepas's own PORTAL_ID (SagePasSync.js) -- the bridge rejects
/// any other portal_id outright.
pub(crate) const BRIDGE_PORTAL_ID: i64 = 51752298;

pub(crate) async fn find_or_create_contact(
    state: &AppState,
    email: &str,
    name: &str,
    phone: &str,
) -> Result<String, (StatusCode, String)> {
    let bridge_url = state.hubspot_bridge_url.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "HubSpot bridge is not configured".to_string(),
        )
    })?;
    let bridge_secret = state.hubspot_bridge_secret.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "HubSpot bridge is not configured".to_string(),
        )
    })?;

    let (firstname, lastname) = match name.split_once(' ') {
        Some((first, rest)) => (first.to_string(), rest.trim().to_string()),
        None => (name.to_string(), String::new()),
    };

    let resp = state
        .hubspot_http_client
        .post(bridge_url)
        .bearer_auth(bridge_secret)
        .json(&serde_json::json!({
            "operation": "find_or_create_contact_by_email",
            "email": email,
            "properties": {
                "firstname": firstname,
                "lastname": lastname,
                "phone": phone,
            },
        }))
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("HubSpot bridge request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("HubSpot bridge returned {status}: {body}"),
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("invalid bridge response: {e}")))?;
    body["objectId"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "bridge response missing objectId".to_string(),
            )
        })
}

/// Resolves the contact for `email` and upserts the
/// `(portal_id, 'contact', object_id) -> customer_id` link row inside the
/// caller's already-open transaction. Returns the resolved HubSpot object_id.
pub(crate) async fn ensure_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    customer_id: Uuid,
    email: &str,
    name: &str,
    phone: &str,
) -> Result<String, (StatusCode, String)> {
    let object_id = find_or_create_contact(state, email, name, phone).await?;

    sqlx::query(
        r#"
        INSERT INTO hubspot_record_links (portal_id, object_type, object_id, customer_id)
        VALUES ($1, 'contact', $2, $3)
        ON CONFLICT (portal_id, object_type, object_id) DO UPDATE SET
            customer_id = EXCLUDED.customer_id,
            updated_at = NOW()
        "#,
    )
    .bind(BRIDGE_PORTAL_ID)
    .bind(&object_id)
    .bind(customer_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(object_id)
}
