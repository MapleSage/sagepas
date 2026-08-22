use std::time::Duration as StdDuration;

use axum::{
    Json,
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, Duration, Utc};
use domain::auth::PasRole;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;

const STAFF_ROLES: &[PasRole] = &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];
const ALLOWED_OBJECT_TYPES: &[&str] = &["contact", "company", "deal", "ticket"];
const BRIDGE_SECRET_HEADER: &str = "x-sagepas-bridge-secret";
const MAX_OBJECT_ID_BYTES: usize = 512;
const MAX_PROPERTIES_BYTES: usize = 128 * 1024;
const MAX_PROPERTY_KEYS: usize = 500;
const MAX_PROPERTY_DEPTH: usize = 8;
const MAX_PROPERTY_STRING_BYTES: usize = 8 * 1024;
const MAX_DISPATCH_ATTEMPTS: i32 = 10;
const DISPATCH_CLAIM_TIMEOUT_MINUTES: i64 = 5;
const DISPATCH_IDLE_SECONDS: u64 = 5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertHubSpotContextRequest {
    pub customer_id: Option<Uuid>,
    pub quote_id: Option<Uuid>,
    pub policy_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct HubSpotContextResponse {
    pub portal_id: i64,
    pub object_type: String,
    pub object_id: String,
    pub customer: Option<CustomerSummary>,
    pub quote: Option<QuoteSummary>,
    pub policy: Option<PolicySummary>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CustomerSummary {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub country: String,
}

#[derive(Debug, Serialize)]
pub struct QuoteSummary {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub state: String,
    pub premium: f64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PolicySummary {
    pub id: Uuid,
    pub quote_id: Uuid,
    pub policy_number: String,
    pub customer_id: Uuid,
    pub state: String,
    pub premium: f64,
    pub currency: String,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ContextRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    customer_id: Option<Uuid>,
    customer_name: Option<String>,
    customer_email: Option<String>,
    customer_phone: Option<String>,
    customer_country: Option<String>,
    quote_id: Option<Uuid>,
    quote_customer_id: Option<Uuid>,
    quote_state: Option<String>,
    quote_premium: Option<f64>,
    quote_currency: Option<String>,
    quote_created_at: Option<DateTime<Utc>>,
    quote_updated_at: Option<DateTime<Utc>>,
    policy_id: Option<Uuid>,
    policy_quote_id: Option<Uuid>,
    policy_number: Option<String>,
    policy_customer_id: Option<Uuid>,
    policy_state: Option<String>,
    policy_premium: Option<f64>,
    policy_currency: Option<String>,
    policy_start_date: Option<DateTime<Utc>>,
    policy_end_date: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct SyncStatusRow {
    status: Option<String>,
    last_event_id: Option<Uuid>,
    attempt_count: Option<i32>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_synced_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    sync_updated_at: Option<DateTime<Utc>>,
    cached_properties: Option<Value>,
    source_updated_at: Option<DateTime<Utc>>,
    cached_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct HubSpotSyncStatusResponse {
    pub portal_id: i64,
    pub object_type: String,
    pub object_id: String,
    pub status: String,
    pub last_event_id: Option<Uuid>,
    pub attempt_count: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub sync_updated_at: Option<DateTime<Utc>>,
    pub cached_properties: Value,
    pub source_updated_at: Option<DateTime<Utc>>,
    pub cached_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct RetrySyncResponse {
    pub event_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheInboundPropertiesRequest {
    pub properties: Map<String, Value>,
    pub source_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CacheInboundPropertiesResponse {
    pub accepted: bool,
    pub cached_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxEvent {
    id: Uuid,
    portal_id: i64,
    object_type: String,
    object_id: String,
    operation: String,
    payload: Value,
    attempts: i32,
}

#[derive(Debug, Serialize)]
struct BridgeDispatchRequest<'a> {
    event_id: Uuid,
    portal_id: i64,
    object_type: &'a str,
    object_id: &'a str,
    operation: &'a str,
    payload: &'a Value,
}

/// GET /api/v1/hubspot/context/:portal_id/:object_type/:object_id
///
/// Returns an empty context (explicit null native records) when the HubSpot
/// object has not been linked yet.
pub async fn get_context(
    Path((portal_id, object_type, object_id)): Path<(i64, String, String)>,
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<HubSpotContextResponse>, (StatusCode, String)> {
    let (object_type, object_id) = validate_identity(portal_id, &object_type, &object_id)?;

    let row = sqlx::query_as::<_, ContextRow>(&context_select("hubspot_record_links"))
        .bind(portal_id)
        .bind(&object_type)
        .bind(&object_id)
        .fetch_optional(&**state.db)
        .await
        .map_err(database_error)?;

    Ok(Json(context_response(
        portal_id,
        object_type,
        object_id,
        row,
    )))
}

/// PUT /api/v1/hubspot/context/:portal_id/:object_type/:object_id
///
/// Commits the native mapping and its credential-free outbound snapshot in the
/// same database transaction. No HubSpot or bridge request occurs here.
pub async fn upsert_context(
    Path((portal_id, object_type, object_id)): Path<(i64, String, String)>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpsertHubSpotContextRequest>,
) -> Result<Json<HubSpotContextResponse>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    let (object_type, object_id) = validate_identity(portal_id, &object_type, &object_id)?;

    let query = format!(
        r#"
        WITH upserted AS (
            INSERT INTO hubspot_record_links
                (portal_id, object_type, object_id, customer_id, quote_id, policy_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (portal_id, object_type, object_id) DO UPDATE SET
                customer_id = EXCLUDED.customer_id,
                quote_id = EXCLUDED.quote_id,
                policy_id = EXCLUDED.policy_id,
                updated_at = NOW()
            RETURNING *
        )
        {}
        "#,
        context_select("upserted")
    );

    let mut tx = state.db.begin().await.map_err(database_error)?;
    let row = sqlx::query_as::<_, ContextRow>(&query)
        .bind(portal_id)
        .bind(&object_type)
        .bind(&object_id)
        .bind(req.customer_id)
        .bind(req.quote_id)
        .bind(req.policy_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;

    state.hubspot_dispatch_notify.notify_one();
    Ok(Json(context_response(
        portal_id,
        object_type,
        object_id,
        Some(row),
    )))
}

/// GET /api/v1/hubspot/sync/:portal_id/:object_type/:object_id
pub async fn get_sync_status(
    Path((portal_id, object_type, object_id)): Path<(i64, String, String)>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<HubSpotSyncStatusResponse>, (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    let (object_type, object_id) = validate_identity(portal_id, &object_type, &object_id)?;

    let row = sqlx::query_as::<_, SyncStatusRow>(
        r#"
        SELECT
            sync.status,
            sync.last_event_id,
            sync.attempt_count,
            sync.last_attempt_at,
            sync.last_synced_at,
            sync.last_error,
            sync.updated_at AS sync_updated_at,
            cache.properties AS cached_properties,
            cache.source_updated_at,
            cache.cached_at
        FROM hubspot_record_links link
        LEFT JOIN hubspot_sync_state sync
          ON sync.portal_id = link.portal_id
         AND sync.object_type = link.object_type
         AND sync.object_id = link.object_id
        LEFT JOIN hubspot_crm_property_cache cache
          ON cache.portal_id = link.portal_id
         AND cache.object_type = link.object_type
         AND cache.object_id = link.object_id
        WHERE link.portal_id = $1
          AND link.object_type = $2
          AND link.object_id = $3
        "#,
    )
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .fetch_optional(&**state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(link_not_found)?;

    let cached_properties = sanitize_properties(row.cached_properties);
    Ok(Json(HubSpotSyncStatusResponse {
        portal_id,
        object_type,
        object_id,
        status: row.status.unwrap_or_else(|| "not_queued".to_string()),
        last_event_id: row.last_event_id,
        attempt_count: row.attempt_count.unwrap_or(0),
        last_attempt_at: row.last_attempt_at,
        last_synced_at: row.last_synced_at,
        last_error: row.last_error,
        sync_updated_at: row.sync_updated_at,
        cached_properties,
        source_updated_at: row.source_updated_at,
        cached_at: row.cached_at,
    }))
}

/// POST /api/v1/hubspot/sync/:portal_id/:object_type/:object_id/retry
///
/// A retry is another durable snapshot. It does not wait for or depend on the
/// bridge being configured or reachable.
pub async fn retry_sync(
    Path((portal_id, object_type, object_id)): Path<(i64, String, String)>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, Json<RetrySyncResponse>), (StatusCode, String)> {
    require_roles(&user, STAFF_ROLES)?;
    let (object_type, object_id) = validate_identity(portal_id, &object_type, &object_id)?;

    let mut tx = state.db.begin().await.map_err(database_error)?;
    let event_id = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT hubspot_enqueue_sync($1, $2, $3, 'manual_retry')",
    )
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?
    .ok_or_else(link_not_found)?;

    sqlx::query(
        r#"
        INSERT INTO hubspot_sync_audit (
            event_id, portal_id, object_type, object_id, direction, outcome, details
        ) VALUES ($1, $2, $3, $4, 'outbound', 'retry_requested', '{}'::jsonb)
        "#,
    )
    .bind(event_id)
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;

    state.hubspot_dispatch_notify.notify_one();
    Ok((
        StatusCode::ACCEPTED,
        Json(RetrySyncResponse {
            event_id,
            status: "pending",
        }),
    ))
}

/// Route middleware for the internal callback. This route intentionally sits
/// outside bearer authentication and has its own dedicated shared-secret
/// boundary. Missing server configuration fails closed.
pub async fn require_bridge_secret(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let expected = state.hubspot_bridge_secret.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "HubSpot bridge callback is not configured".to_string(),
        )
    })?;

    let presented = request
        .headers()
        .get(BRIDGE_SECRET_HEADER)
        .map(|value| value.as_bytes())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "invalid bridge authentication".to_string(),
            )
        })?;

    if !constant_time_eq(presented, expected.as_bytes()) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid bridge authentication".to_string(),
        ));
    }

    Ok(next.run(request).await)
}

/// POST /api/v1/internal/hubspot/cache/:portal_id/:object_type/:object_id
pub async fn cache_inbound_properties(
    Path((portal_id, object_type, object_id)): Path<(i64, String, String)>,
    State(state): State<AppState>,
    Json(req): Json<CacheInboundPropertiesRequest>,
) -> Result<(StatusCode, Json<CacheInboundPropertiesResponse>), (StatusCode, String)> {
    let (object_type, object_id) = validate_identity(portal_id, &object_type, &object_id)?;
    validate_properties(&req.properties)?;
    let properties = Value::Object(req.properties);

    let mut tx = state.db.begin().await.map_err(database_error)?;
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM hubspot_record_links
            WHERE portal_id = $1 AND object_type = $2 AND object_id = $3
        )
        "#,
    )
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if !exists {
        return Err(link_not_found());
    }

    let cached_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        r#"
        INSERT INTO hubspot_crm_property_cache (
            portal_id, object_type, object_id, properties, source_updated_at, cached_at
        ) VALUES ($1, $2, $3, $4, $5, NOW())
        ON CONFLICT (portal_id, object_type, object_id) DO UPDATE SET
            properties = EXCLUDED.properties,
            source_updated_at = EXCLUDED.source_updated_at,
            cached_at = NOW()
        WHERE EXCLUDED.source_updated_at IS NULL
           OR hubspot_crm_property_cache.source_updated_at IS NULL
           OR EXCLUDED.source_updated_at >= hubspot_crm_property_cache.source_updated_at
        RETURNING cached_at
        "#,
    )
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .bind(&properties)
    .bind(req.source_updated_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;

    let outcome = if cached_at.is_some() {
        "properties_cached"
    } else {
        "properties_ignored"
    };
    sqlx::query(
        r#"
        INSERT INTO hubspot_sync_audit (
            portal_id, object_type, object_id, direction, outcome, details
        ) VALUES (
            $1, $2, $3, 'inbound', $4,
            jsonb_build_object('property_count', $5, 'has_source_timestamp', $6)
        )
        "#,
    )
    .bind(portal_id)
    .bind(&object_type)
    .bind(&object_id)
    .bind(outcome)
    .bind(properties.as_object().map_or(0_i32, |map| map.len() as i32))
    .bind(req.source_updated_at.is_some())
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(CacheInboundPropertiesResponse {
            accepted: cached_at.is_some(),
            cached_at,
        }),
    ))
}

/// Runs the durable outbox dispatcher. It is deliberately separate from every
/// native PAS transaction. Multiple API replicas may run this safely because
/// claims use `FOR UPDATE SKIP LOCKED` and per-record ordering.
pub async fn run_outbox_dispatcher(state: AppState) {
    let Some(bridge_url) = configured_bridge_url(state.hubspot_bridge_url.as_deref()) else {
        warn!("HubSpot outbox dispatcher disabled: bridge URL is unset or invalid");
        return;
    };
    let Some(bridge_secret) = state.hubspot_bridge_secret.clone() else {
        warn!("HubSpot outbox dispatcher disabled: bridge secret is unset");
        return;
    };

    info!("HubSpot durable outbox dispatcher started");
    loop {
        match claim_next_event(&state).await {
            Ok(Some(event)) => {
                dispatch_event(&state, &bridge_url, &bridge_secret, event).await;
            }
            Ok(None) => {
                tokio::select! {
                    _ = tokio::time::sleep(StdDuration::from_secs(DISPATCH_IDLE_SECONDS)) => {},
                    _ = state.hubspot_dispatch_notify.notified() => {},
                }
            }
            Err(err) => {
                error!(error = %err, "failed to claim HubSpot outbox event");
                tokio::time::sleep(StdDuration::from_secs(DISPATCH_IDLE_SECONDS)).await;
            }
        }
    }
}

async fn claim_next_event(state: &AppState) -> Result<Option<OutboxEvent>, sqlx::Error> {
    let mut tx = state.db.begin().await?;
    let event = sqlx::query_as::<_, OutboxEvent>(
        r#"
        WITH candidate AS (
            SELECT candidate.id
            FROM hubspot_sync_outbox candidate
            WHERE candidate.attempts < $1
              AND (
                    (candidate.status IN ('pending', 'failed') AND candidate.available_at <= NOW())
                 OR (candidate.status = 'dispatching'
                     AND candidate.claimed_at < NOW() - ($2 * INTERVAL '1 minute'))
              )
              AND NOT EXISTS (
                  SELECT 1
                  FROM hubspot_sync_outbox earlier
                  WHERE earlier.portal_id = candidate.portal_id
                    AND earlier.object_type = candidate.object_type
                    AND earlier.object_id = candidate.object_id
                    AND (earlier.created_at, earlier.id) < (candidate.created_at, candidate.id)
                    AND earlier.attempts < $1
                    AND earlier.status IN ('pending', 'failed', 'dispatching')
              )
            ORDER BY candidate.available_at, candidate.created_at, candidate.id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE hubspot_sync_outbox outbox
        SET status = 'dispatching',
            attempts = outbox.attempts + 1,
            claimed_at = NOW(),
            completed_at = NULL,
            last_error = NULL,
            updated_at = NOW()
        FROM candidate
        WHERE outbox.id = candidate.id
        RETURNING outbox.id, outbox.portal_id, outbox.object_type, outbox.object_id,
                  outbox.operation, outbox.payload, outbox.attempts
        "#,
    )
    .bind(MAX_DISPATCH_ATTEMPTS)
    .bind(DISPATCH_CLAIM_TIMEOUT_MINUTES)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(event) = &event {
        sqlx::query(
            r#"
            UPDATE hubspot_sync_state
            SET status = 'dispatching',
                attempt_count = $2,
                last_attempt_at = NOW(),
                last_error = NULL,
                updated_at = NOW()
            WHERE last_event_id = $1
            "#,
        )
        .bind(event.id)
        .bind(event.attempts)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO hubspot_sync_audit (
                event_id, portal_id, object_type, object_id, direction, outcome, details
            ) VALUES (
                $1, $2, $3, $4, 'outbound', 'dispatching',
                jsonb_build_object('attempt', $5)
            )
            "#,
        )
        .bind(event.id)
        .bind(event.portal_id)
        .bind(&event.object_type)
        .bind(&event.object_id)
        .bind(event.attempts)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(event)
}

async fn dispatch_event(
    state: &AppState,
    bridge_url: &reqwest::Url,
    bridge_secret: &str,
    event: OutboxEvent,
) {
    let request = BridgeDispatchRequest {
        event_id: event.id,
        portal_id: event.portal_id,
        object_type: &event.object_type,
        object_id: &event.object_id,
        operation: &event.operation,
        payload: &event.payload,
    };

    let result = hubspot_bridge_request(&state.hubspot_http_client, bridge_url.as_str(), bridge_secret)
        .json(&request)
        .send()
        .await;

    match result {
        Ok(response) if response.status().is_success() => {
            if let Err(err) = mark_dispatch_succeeded(state, &event).await {
                error!(event_id = %event.id, error = %err, "failed to complete HubSpot outbox event");
            }
        }
        Ok(response) => {
            let status = response.status();
            let retryable = status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
            let code = format!("http_{}", status.as_u16());
            let message = format!("bridge returned HTTP {}", status.as_u16());
            if let Err(err) = mark_dispatch_failed(state, &event, &code, &message, retryable).await
            {
                error!(event_id = %event.id, error = %err, "failed to record HubSpot dispatch failure");
            }
        }
        Err(err) => {
            let code = if err.is_timeout() {
                "timeout"
            } else if err.is_connect() {
                "connect"
            } else if err.is_builder() {
                "invalid_request"
            } else {
                "transport"
            };
            let retryable = !err.is_builder();
            let message = format!("bridge dispatch failed ({code})");
            if let Err(db_err) =
                mark_dispatch_failed(state, &event, code, &message, retryable).await
            {
                error!(event_id = %event.id, error = %db_err, "failed to record HubSpot dispatch failure");
            }
        }
    }
}

fn hubspot_bridge_request(
    client: &reqwest::Client,
    bridge_url: &str,
    bridge_secret: &str,
) -> reqwest::RequestBuilder {
    // The deployed HubSpot function accepts this shared secret as a standard
    // bearer credential. Keep the custom bridge header reserved for
    // HubSpot-to-PAS inbound cache writes.
    client.post(bridge_url).bearer_auth(bridge_secret)
}

async fn mark_dispatch_succeeded(state: &AppState, event: &OutboxEvent) -> Result<(), sqlx::Error> {
    let mut tx = state.db.begin().await?;
    let completed = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE hubspot_sync_outbox
        SET status = 'succeeded', completed_at = NOW(), last_error = NULL, updated_at = NOW()
        WHERE id = $1 AND status = 'dispatching' AND attempts = $2
        RETURNING id
        "#,
    )
    .bind(event.id)
    .bind(event.attempts)
    .fetch_optional(&mut *tx)
    .await?;
    if completed.is_none() {
        tx.rollback().await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE hubspot_sync_state
        SET status = CASE WHEN last_event_id = $1 THEN 'succeeded' ELSE status END,
            last_synced_at = NOW(),
            last_error = CASE WHEN last_event_id = $1 THEN NULL ELSE last_error END,
            updated_at = NOW()
        WHERE portal_id = $2 AND object_type = $3 AND object_id = $4
        "#,
    )
    .bind(event.id)
    .bind(event.portal_id)
    .bind(&event.object_type)
    .bind(&event.object_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO hubspot_sync_audit (
            event_id, portal_id, object_type, object_id, direction, outcome, details
        ) VALUES ($1, $2, $3, $4, 'outbound', 'succeeded', '{}'::jsonb)
        "#,
    )
    .bind(event.id)
    .bind(event.portal_id)
    .bind(&event.object_type)
    .bind(&event.object_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn mark_dispatch_failed(
    state: &AppState,
    event: &OutboxEvent,
    error_code: &str,
    message: &str,
    response_retryable: bool,
) -> Result<(), sqlx::Error> {
    let retryable = response_retryable && event.attempts < MAX_DISPATCH_ATTEMPTS;
    let available_at = Utc::now() + Duration::seconds(retry_backoff_seconds(event.attempts));
    let mut tx = state.db.begin().await?;
    let completed = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE hubspot_sync_outbox
        SET status = 'failed', available_at = $2, completed_at = NULL,
            last_error = $3,
            attempts = CASE WHEN $4 THEN attempts ELSE $5 END,
            updated_at = NOW()
        WHERE id = $1 AND status = 'dispatching' AND attempts = $6
        RETURNING id
        "#,
    )
    .bind(event.id)
    .bind(available_at)
    .bind(message)
    .bind(retryable)
    .bind(MAX_DISPATCH_ATTEMPTS)
    .bind(event.attempts)
    .fetch_optional(&mut *tx)
    .await?;
    if completed.is_none() {
        tx.rollback().await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE hubspot_sync_state
        SET status = 'failed', last_error = $2, updated_at = NOW()
        WHERE last_event_id = $1
        "#,
    )
    .bind(event.id)
    .bind(message)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO hubspot_sync_audit (
            event_id, portal_id, object_type, object_id, direction, outcome,
            error_code, details
        ) VALUES (
            $1, $2, $3, $4, 'outbound', 'failed', $5,
            jsonb_build_object('attempt', $6, 'retryable', $7)
        )
        "#,
    )
    .bind(event.id)
    .bind(event.portal_id)
    .bind(&event.object_type)
    .bind(&event.object_id)
    .bind(error_code)
    .bind(event.attempts)
    .bind(retryable)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if retryable {
        state.hubspot_dispatch_notify.notify_one();
    }
    Ok(())
}

fn validate_identity(
    portal_id: i64,
    object_type: &str,
    object_id: &str,
) -> Result<(String, String), (StatusCode, String)> {
    if portal_id <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "portal_id must be positive".to_string(),
        ));
    }

    let object_type = object_type.trim().to_ascii_lowercase();
    if !ALLOWED_OBJECT_TYPES.contains(&object_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "object_type must be one of: contact, company, deal, ticket".to_string(),
        ));
    }

    let object_id = object_id.trim().to_string();
    if object_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "object_id must not be empty".to_string(),
        ));
    }
    if object_id.len() > MAX_OBJECT_ID_BYTES || object_id.chars().any(char::is_control) {
        return Err((
            StatusCode::BAD_REQUEST,
            "object_id is invalid or too long".to_string(),
        ));
    }

    Ok((object_type, object_id))
}

fn validate_properties(properties: &Map<String, Value>) -> Result<(), (StatusCode, String)> {
    if properties.len() > MAX_PROPERTY_KEYS {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "too many cached properties".to_string(),
        ));
    }
    let value = Value::Object(properties.clone());
    if serde_json::to_vec(&value).map_or(true, |encoded| encoded.len() > MAX_PROPERTIES_BYTES) {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "cached properties payload is too large".to_string(),
        ));
    }
    validate_property_value(&value, 0)
}

fn validate_property_value(value: &Value, depth: usize) -> Result<(), (StatusCode, String)> {
    if depth > MAX_PROPERTY_DEPTH {
        return Err((
            StatusCode::BAD_REQUEST,
            "cached properties are nested too deeply".to_string(),
        ));
    }
    match value {
        Value::String(value) if value.len() > MAX_PROPERTY_STRING_BYTES => Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "cached property value is too large".to_string(),
        )),
        Value::Array(values) => {
            for value in values {
                validate_property_value(value, depth + 1)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                if is_sensitive_property_key(key) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "credential-like properties are not accepted".to_string(),
                    ));
                }
                validate_property_value(value, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_sensitive_property_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
    [
        "token",
        "secret",
        "password",
        "authorization",
        "credential",
        "api_key",
        "apikey",
    ]
    .iter()
    .any(|term| normalized.contains(term))
}

fn sanitize_properties(properties: Option<Value>) -> Value {
    fn sanitize(value: Value) -> Value {
        match value {
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .filter(|(key, _)| !is_sensitive_property_key(key))
                    .map(|(key, value)| (key, sanitize(value)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
            other => other,
        }
    }

    properties
        .map(sanitize)
        .unwrap_or_else(|| Value::Object(Map::new()))
}

fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    let max_len = presented.len().max(expected.len());
    let mut difference = presented.len() ^ expected.len();
    for index in 0..max_len {
        let left = presented.get(index).copied().unwrap_or(0);
        let right = expected.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

fn configured_bridge_url(raw: Option<&str>) -> Option<reqwest::Url> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let url = reqwest::Url::parse(raw).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    (secure || loopback_http).then_some(url)
}

fn retry_backoff_seconds(attempt: i32) -> i64 {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(0).min(8);
    (5_i64.saturating_mul(1_i64 << exponent)).min(300)
}

fn context_select(source: &str) -> String {
    format!(
        r#"
        SELECT
            link.created_at,
            link.updated_at,
            customer.id AS customer_id,
            customer.name AS customer_name,
            customer.email AS customer_email,
            customer.phone AS customer_phone,
            customer.country AS customer_country,
            quote.id AS quote_id,
            quote.customer_id AS quote_customer_id,
            quote.state AS quote_state,
            quote.premium AS quote_premium,
            quote.currency AS quote_currency,
            quote.created_at AS quote_created_at,
            quote.updated_at AS quote_updated_at,
            policy.id AS policy_id,
            policy.quote_id AS policy_quote_id,
            policy.policy_number,
            policy.customer_id AS policy_customer_id,
            policy.state AS policy_state,
            policy.premium AS policy_premium,
            policy.currency AS policy_currency,
            policy.start_date AS policy_start_date,
            policy.end_date AS policy_end_date
        FROM {source} link
        LEFT JOIN customers customer ON customer.id = link.customer_id
        LEFT JOIN quotes quote ON quote.id = link.quote_id
        LEFT JOIN policies policy ON policy.id = link.policy_id
        WHERE link.portal_id = $1
          AND link.object_type = $2
          AND link.object_id = $3
        "#
    )
}

fn context_response(
    portal_id: i64,
    object_type: String,
    object_id: String,
    row: Option<ContextRow>,
) -> HubSpotContextResponse {
    let Some(row) = row else {
        return HubSpotContextResponse {
            portal_id,
            object_type,
            object_id,
            customer: None,
            quote: None,
            policy: None,
            created_at: None,
            updated_at: None,
        };
    };

    let customer = row.customer_id.map(|id| CustomerSummary {
        id,
        name: row.customer_name.unwrap_or_default(),
        email: row.customer_email.unwrap_or_default(),
        phone: row.customer_phone.unwrap_or_default(),
        country: row.customer_country.unwrap_or_default(),
    });
    let quote = row.quote_id.map(|id| QuoteSummary {
        id,
        customer_id: row.quote_customer_id.expect("joined quote has customer_id"),
        state: row.quote_state.unwrap_or_default(),
        premium: row.quote_premium.unwrap_or_default(),
        currency: row.quote_currency.unwrap_or_default(),
        created_at: row.quote_created_at.expect("joined quote has created_at"),
        updated_at: row.quote_updated_at.expect("joined quote has updated_at"),
    });
    let policy = row.policy_id.map(|id| PolicySummary {
        id,
        quote_id: row.policy_quote_id.expect("joined policy has quote_id"),
        policy_number: row.policy_number.unwrap_or_default(),
        customer_id: row
            .policy_customer_id
            .expect("joined policy has customer_id"),
        state: row.policy_state.unwrap_or_default(),
        premium: row.policy_premium.unwrap_or_default(),
        currency: row.policy_currency.unwrap_or_default(),
        start_date: row.policy_start_date.expect("joined policy has start_date"),
        end_date: row.policy_end_date.expect("joined policy has end_date"),
    });

    HubSpotContextResponse {
        portal_id,
        object_type,
        object_id,
        customer,
        quote,
        policy,
        created_at: Some(row.created_at),
        updated_at: Some(row.updated_at),
    }
}

#[derive(Debug, Serialize)]
pub struct BackfillLinksResponse {
    pub unlinked_before: usize,
    pub linked: Vec<Uuid>,
    pub skipped_ambiguous_email: Vec<AmbiguousEmailGroup>,
    pub skipped_blank_email: Vec<Uuid>,
    pub errors: Vec<BackfillLinkError>,
}

#[derive(Debug, Serialize)]
pub struct AmbiguousEmailGroup {
    pub email: String,
    pub customer_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct BackfillLinkError {
    pub customer_id: Uuid,
    pub error: String,
}

/// One unlinked customer row, as read for the backfill (work order item 8).
struct UnlinkedCustomer {
    id: Uuid,
    email: String,
    name: String,
    phone: String,
}

/// Splits unlinked customers into: candidates safe to auto-link (exactly one
/// customer for their normalized email), ambiguous groups (more than one
/// customer shares an email -- the link schema allows only one customer_id
/// per HubSpot object_id, so guessing which one wins would misattribute a
/// real contact), and blank-email rows. Pure and unit-tested separately from
/// the DB/bridge IO in `backfill_hubspot_links` below.
fn partition_unlinked_customers(
    rows: Vec<UnlinkedCustomer>,
) -> (Vec<UnlinkedCustomer>, Vec<AmbiguousEmailGroup>, Vec<Uuid>) {
    let mut blank_email = Vec::new();
    let mut by_email: std::collections::HashMap<String, Vec<UnlinkedCustomer>> =
        std::collections::HashMap::new();

    for row in rows {
        let normalized = row.email.trim().to_lowercase();
        if normalized.is_empty() {
            blank_email.push(row.id);
            continue;
        }
        by_email.entry(normalized).or_default().push(row);
    }

    let mut candidates = Vec::new();
    let mut ambiguous = Vec::new();
    for (email, mut group) in by_email {
        if group.len() > 1 {
            ambiguous.push(AmbiguousEmailGroup {
                email,
                customer_ids: group.iter().map(|r| r.id).collect(),
            });
        } else {
            candidates.push(group.remove(0));
        }
    }

    (candidates, ambiguous, blank_email)
}

/// POST /api/v1/admin/hubspot/backfill-links -- Admin only, one-time
/// reconciliation (work order item 8). Every PAS customer should get a
/// hubspot_record_links row automatically as of item 3; this catches
/// customers created before that fix landed (or by any future path that
/// still manages to skip it). Never guesses: ambiguous or blank emails are
/// reported, not linked.
pub async fn backfill_hubspot_links(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<BackfillLinksResponse>, (StatusCode, String)> {
    require_roles(&user, &[PasRole::Admin])?;

    let rows: Vec<UnlinkedCustomer> = sqlx::query_as::<_, (Uuid, String, String, String)>(
        r#"
        SELECT c.id, c.email, c.name, c.phone
        FROM customers c
        WHERE NOT EXISTS (
            SELECT 1 FROM hubspot_record_links l WHERE l.customer_id = c.id
        )
        ORDER BY c.created_at
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, email, name, phone)| UnlinkedCustomer {
        id,
        email,
        name,
        phone,
    })
    .collect();

    let unlinked_before = rows.len();
    let (candidates, skipped_ambiguous_email, skipped_blank_email) =
        partition_unlinked_customers(rows);

    let mut linked = Vec::new();
    let mut errors = Vec::new();

    for candidate in candidates {
        let mut tx = match state.db.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                errors.push(BackfillLinkError {
                    customer_id: candidate.id,
                    error: e.to_string(),
                });
                continue;
            }
        };

        let link_result = crate::hubspot_bridge::ensure_link(
            &mut tx,
            &state,
            candidate.id,
            &candidate.email,
            &candidate.name,
            &candidate.phone,
        )
        .await;

        match link_result {
            Ok(_) => match tx.commit().await {
                Ok(_) => linked.push(candidate.id),
                Err(e) => errors.push(BackfillLinkError {
                    customer_id: candidate.id,
                    error: e.to_string(),
                }),
            },
            Err((_, message)) => errors.push(BackfillLinkError {
                customer_id: candidate.id,
                error: message,
            }),
        }
    }

    Ok(Json(BackfillLinksResponse {
        unlinked_before,
        linked,
        skipped_ambiguous_email,
        skipped_blank_email,
        errors,
    }))
}

fn link_not_found() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        "HubSpot record mapping was not found".to_string(),
    )
}

fn database_error(err: sqlx::Error) -> (StatusCode, String) {
    if let sqlx::Error::Database(db_err) = &err
        && db_err.code().as_deref() == Some("23503")
    {
        return (
            StatusCode::BAD_REQUEST,
            "one or more linked SagePAS records do not exist".to_string(),
        );
    }

    error!(error = %err, "HubSpot synchronization database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "unable to load or update HubSpot synchronization state".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlinked(id: Uuid, email: &str) -> UnlinkedCustomer {
        UnlinkedCustomer {
            id,
            email: email.to_string(),
            name: "Test Customer".to_string(),
            phone: "+15550000000".to_string(),
        }
    }

    #[test]
    fn unique_emails_become_link_candidates() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let (candidates, ambiguous, blank) = partition_unlinked_customers(vec![
            unlinked(a, "Alice@Example.com"),
            unlinked(b, "bob@example.com"),
        ]);
        assert_eq!(candidates.len(), 2);
        assert!(ambiguous.is_empty());
        assert!(blank.is_empty());
    }

    #[test]
    fn duplicate_normalized_email_is_reported_not_guessed() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let (candidates, ambiguous, blank) = partition_unlinked_customers(vec![
            unlinked(a, "Same@Example.com"),
            unlinked(b, " same@example.com "),
        ]);
        assert!(candidates.is_empty());
        assert!(blank.is_empty());
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].email, "same@example.com");
        let mut ids = ambiguous[0].customer_ids.clone();
        ids.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn blank_or_whitespace_only_email_is_skipped_not_guessed() {
        let a = Uuid::new_v4();
        let (candidates, ambiguous, blank) =
            partition_unlinked_customers(vec![unlinked(a, "   ")]);
        assert!(candidates.is_empty());
        assert!(ambiguous.is_empty());
        assert_eq!(blank, vec![a]);
    }

    #[test]
    fn accepts_and_normalizes_valid_identity() {
        let identity = validate_identity(123, " Deal ", " 456 ").unwrap();
        assert_eq!(identity, ("deal".to_string(), "456".to_string()));
    }

    #[test]
    fn rejects_secret_or_other_unknown_request_fields() {
        let request = serde_json::json!({
            "customer_id": null,
            "access_token": "must-not-be-accepted"
        });
        assert!(serde_json::from_value::<UpsertHubSpotContextRequest>(request).is_err());
    }

    #[test]
    fn rejects_non_positive_portal_id() {
        let err = validate_identity(0, "contact", "1").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(err.1, "portal_id must be positive");
    }

    #[test]
    fn rejects_unknown_object_type() {
        let err = validate_identity(1, "owner", "1").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("contact, company, deal, ticket"));
    }

    #[test]
    fn rejects_blank_or_oversized_object_id() {
        assert_eq!(
            validate_identity(1, "ticket", "   ").unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_identity(1, "ticket", &"x".repeat(MAX_OBJECT_ID_BYTES + 1))
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn rejects_credential_like_cached_properties_at_any_depth() {
        let properties = serde_json::from_value::<Map<String, Value>>(serde_json::json!({
            "safe": {"refresh-token": "must-not-be-stored"}
        }))
        .unwrap();
        let err = validate_properties(&properties).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn accepts_bounded_credential_free_cached_properties() {
        let properties = serde_json::from_value::<Map<String, Value>>(serde_json::json!({
            "lifecyclestage": "customer",
            "annualrevenue": 120000,
            "active": true
        }))
        .unwrap();
        validate_properties(&properties).unwrap();
    }

    #[test]
    fn secret_comparison_handles_equal_unequal_and_different_lengths() {
        assert!(constant_time_eq(
            b"a-long-bridge-secret",
            b"a-long-bridge-secret"
        ));
        assert!(!constant_time_eq(
            b"a-long-bridge-secret",
            b"a-long-bridge-secreu"
        ));
        assert!(!constant_time_eq(b"short", b"a-long-bridge-secret"));
    }

    #[test]
    fn bridge_url_rejects_cleartext_remote_and_embedded_credentials() {
        assert!(configured_bridge_url(None).is_none());
        assert!(configured_bridge_url(Some("http://bridge.internal/sync")).is_none());
        assert!(configured_bridge_url(Some("https://user:pass@bridge.example/sync")).is_none());
        assert!(configured_bridge_url(Some("https://bridge.example/sync?token=nope")).is_none());
        assert!(configured_bridge_url(Some("https://bridge.example/sync")).is_some());
        assert!(configured_bridge_url(Some("http://127.0.0.1:3000/sync")).is_some());
    }

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_backoff_seconds(1), 5);
        assert_eq!(retry_backoff_seconds(2), 10);
        assert_eq!(retry_backoff_seconds(10), 300);
        assert_eq!(retry_backoff_seconds(i32::MAX), 300);
    }

    #[test]
    fn outbound_bridge_dispatch_uses_bearer_auth() {
        let request = hubspot_bridge_request(
            &reqwest::Client::new(),
            "https://bridge.example/sync",
            "shared-secret",
        )
        .build()
        .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer shared-secret")
        );
        assert!(request.headers().get(BRIDGE_SECRET_HEADER).is_none());
    }

    #[test]
    fn sanitizes_sensitive_properties_before_staff_response() {
        let value = serde_json::json!({
            "name": "safe",
            "nested": {"authorization": "must-not-be-returned", "stage": "won"}
        });
        let sanitized = sanitize_properties(Some(value));
        assert_eq!(sanitized["name"], "safe");
        assert_eq!(sanitized["nested"]["stage"], "won");
        assert!(sanitized["nested"].get("authorization").is_none());
    }
}
