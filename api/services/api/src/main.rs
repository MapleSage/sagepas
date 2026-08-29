use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware as axum_middleware,
    routing::{get, post},
};
use chrono::Utc;
use composite_event_bus::CompositeEventBus;
use documents::DocumentGenerator;
use event_bus::EventBus;
use event_projector::EventProjector;
use event_router::DefaultRoutePolicy;
use event_store::{EventStore, PostgresEventStore};
use infra::{blob::BlobClient, config::AppConfig, db::DbPool, search::SearchClient};
use oos_orchestrator::OosOrchestrator;
use policy_ledger::{BiTemporalPolicyStore, PolicyLedger};
use policy_lock::{PolicyLockError, PolicyLockManager};
use pricing::PricingEngine;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info, warn};

mod auth_extract;
mod context_providers;
mod handlers;
mod hubspot_bridge;
mod middleware;
mod migration_runner;
mod rate_limit;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let config = Arc::new(AppConfig::from_env().map_err(|e| anyhow::anyhow!("config error: {e}"))?);
    info!(port = config.port, env = %config.node_env, "sagepas api starting");

    let db = Arc::new(DbPool::connect_lazy(&config.database_url)?);
    match migration_runner::run_startup_migrations(&db).await {
        Ok(summary) => {
            info!(
                migration_count = summary.count,
                migrations_applied_now = summary.applied_now,
                migration_files = summary.files.len(),
                "startup migrations applied"
            );
        }
        Err(err) => {
            error!(error = %err, "fatal startup migration failure");
            return Err(err);
        }
    }

    let search = Arc::new(SearchClient::new(
        &config.search_endpoint,
        &config.search_key,
    ));
    let pricing = Arc::new(PricingEngine::new((*search).clone()));
    let subledger = premium_ledger::PremiumSubledger::new(db.clone());

    let blob = Arc::new(BlobClient::from_managed_identity(
        &config.storage_account_name,
        &config.storage_container_name,
    )?);
    let documents = Arc::new(DocumentGenerator::new(
        (*blob).clone(),
        config.storage_container_name.clone(),
        &config.storage_account_name,
    ));

    let entra_staff_validator = if config.entra_configured() {
        info!(tenant = %config.azure_entra_tenant_id, "Entra staff token validation enabled");
        Some(Arc::new(infra::entra_auth::EntraValidator::new(
            config.azure_entra_tenant_id.clone(),
            config.azure_entra_audience.clone(),
            if config.azure_entra_issuer.trim().is_empty() {
                None
            } else {
                Some(config.azure_entra_issuer.clone())
            },
            None,
        )))
    } else {
        warn!(
            "AZURE_ENTRA_TENANT_ID / AZURE_ENTRA_AUDIENCE not configured; Entra staff token validation disabled"
        );
        None
    };
    let entra_consumer_validator = if config.entra_consumer_configured() {
        info!(tenant = %config.azure_entra_consumer_tenant_id, "Entra consumer (CIAM) token validation enabled");
        Some(Arc::new(infra::entra_auth::EntraValidator::new(
            config.azure_entra_consumer_tenant_id.clone(),
            config.azure_entra_consumer_audience.clone(),
            None,
            if config.azure_entra_consumer_authority_host.trim().is_empty() {
                None
            } else {
                Some(config.azure_entra_consumer_authority_host.clone())
            },
        )))
    } else {
        warn!(
            "AZURE_ENTRA_CONSUMER_TENANT_ID / AZURE_ENTRA_CONSUMER_AUDIENCE not configured; consumer/CIAM sign-in disabled"
        );
        None
    };
    let entra_delegate_validator = if config.entra_delegate_configured() {
        info!(
            audience = %config.azure_entra_delegate_audience,
            "Entra delegate (sesure-us staff audience) token validation enabled -- accepted only on the delegated-read path allowlist"
        );
        Some(Arc::new(infra::entra_auth::EntraValidator::new(
            config.azure_entra_tenant_id.clone(),
            config.azure_entra_delegate_audience.clone(),
            None,
            None,
        )))
    } else {
        None
    };
    if !config.entra_configured() && !config.dev_local_auth_enabled {
        warn!(
            "no authentication method is usable: Entra is unconfigured and dev_local_auth_enabled is false"
        );
    }

    let inner_store: Arc<dyn EventStore + Send + Sync> =
        Arc::new(PostgresEventStore::new(db.clone()));
    let event_store_backend = "postgres".to_string();
    let event_bus: Arc<dyn EventBus + Send + Sync> = Arc::new(CompositeEventBus::new(
        Arc::new(DefaultRoutePolicy),
        inner_store.clone(),
    ));

    let timeline_projector = Arc::new(Mutex::new(EventProjector::new()));
    let bootstrap = bootstrap_timeline_projector(&inner_store, &timeline_projector).await?;

    let policy_lock = if config.redis_url.trim().is_empty() {
        Arc::new(PolicyLockManager::new_disabled())
    } else {
        Arc::new(
            PolicyLockManager::new(&config.redis_url).map_err(|err| match err {
                PolicyLockError::RedisError(message) => {
                    anyhow::anyhow!("policy lock init failed: {message}")
                }
                other => anyhow::anyhow!("policy lock init failed: {other}"),
            })?,
        )
    };

    let hubspot_bridge_url = std::env::var("HUBSPOT_BRIDGE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    // Keep the Kubernetes/API name aligned with the HubSpot project secret.
    // HUBSPOT_BRIDGE_SECRET remains a compatibility fallback for the original
    // deployment template, but new deployments use HUBSPOT_SYNC_SECRET.
    let hubspot_bridge_secret = resolve_hubspot_bridge_secret(
        std::env::var("HUBSPOT_SYNC_SECRET").ok(),
        std::env::var("HUBSPOT_BRIDGE_SECRET").ok(),
    );
    if hubspot_bridge_url.is_none() || hubspot_bridge_secret.is_none() {
        warn!(
            bridge_url_configured = hubspot_bridge_url.is_some(),
            bridge_secret_configured = hubspot_bridge_secret.is_some(),
            "HubSpot bridge integration is fail-closed until both environment values are configured"
        );
    }
    // Matches sagepas's own dev/test portal by default -- the value this
    // hardcoded constant always was, before item 9 made it a config change
    // instead of a code change for promotion to a different portal.
    let hubspot_portal_id: i64 = std::env::var("HUBSPOT_PORTAL_ID")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(51752298);
    let hubspot_http_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        // Cloudflare (fronting the bridge at *.hs-sites.com) blocks reqwest's
        // default "reqwest/x.y.z" User-Agent outright (403 managed
        // challenge) -- confirmed live from sagesure-us's identical client,
        // independent of TLS backend. Any non-default UA clears it.
        .user_agent("SageSure-HubSpot-Bridge/1.0")
        .build()?;

    let cu_client = if config.content_understanding_endpoint.trim().is_empty() {
        warn!("CONTENT_UNDERSTANDING_ENDPOINT not configured; FNOL/UW pipeline falls back to GPT-only extraction");
        None
    } else {
        Some(Arc::new(doc_pipeline::content_understanding::ContentUnderstandingClient::new(
            &config.content_understanding_endpoint,
        )))
    };
    let openai_client = Arc::new(infra::openai::OpenAIClient::new(
        &config.azure_openai_endpoint,
        &config.azure_openai_key,
        &config.azure_openai_deployment,
    ));
    let fnol_blob = Arc::new(BlobClient::from_managed_identity(
        &config.storage_account_name,
        "fnol-documents",
    )?);
    let uw_blob = Arc::new(BlobClient::from_managed_identity(
        &config.storage_account_name,
        "uw-documents",
    )?);

    let whatsapp_client = {
        let cfg = whatsapp::WhatsAppConfig {
            access_token: config.whatsapp_access_token.clone(),
            phone_id: config.whatsapp_phone_id.clone(),
        };
        if cfg.is_configured() {
            Some(Arc::new(whatsapp::WhatsAppClient::new(cfg)))
        } else {
            warn!("WHATSAPP_ACCESS_TOKEN/WHATSAPP_PHONE_ID not configured; WhatsApp send disabled");
            None
        }
    };

    let app_state = AppState {
        db: db.clone(),
        config: config.clone(),
        pricing,
        documents,
        ledger: Arc::new(PolicyLedger::new()),
        bitemporal_policy: Arc::new(BiTemporalPolicyStore::new(db.clone())),
        oos_orchestrator: Arc::new(OosOrchestrator::new(db.clone(), config.redis_url.clone())),
        policy_lock,
        event_bus,
        event_store: inner_store,
        timeline_projector,
        event_store_backend,
        projector_bootstrap_event_count: bootstrap.event_count as u64,
        projector_bootstrap_at: bootstrap.replayed_at,
        hubspot_bridge_url,
        hubspot_bridge_secret,
        hubspot_portal_id,
        hubspot_http_client,
        hubspot_dispatch_notify: Arc::new(tokio::sync::Notify::new()),
        entra_staff_validator,
        entra_consumer_validator,
        entra_delegate_validator,
        prospect_rate_limiter: Arc::new(rate_limit::RateLimiter::new(
            handlers::prospect::RATE_LIMIT_MAX_REQUESTS,
            handlers::prospect::RATE_LIMIT_WINDOW,
        )),
        subledger,
        cu_client,
        openai_client,
        fnol_blob,
        uw_blob,
        search,
        conversation: conversation_memory::ConversationStore::new(db.clone()),
        whatsapp_client,
    };

    tokio::spawn(handlers::hubspot::run_outbox_dispatcher(app_state.clone()));

    let authenticated_routes = Router::new()
        .route("/health", get(health))
        .route("/api/v1/health", get(health))
        .route("/api/v1/eventing/status", get(handlers::eventing::status))
        .route("/api/v1/auth/register", post(handlers::auth::register))
        .route("/api/v1/auth/login", post(handlers::auth::login))
        .route("/api/v1/auth/refresh", post(handlers::auth::refresh))
        .route("/api/v1/products", get(handlers::products::list_products))
        .route(
            "/api/v1/customers",
            get(handlers::customers::list_customers).post(handlers::customers::create_customer),
        )
        .route(
            "/api/v1/customers/:id",
            get(handlers::customers::get_customer),
        )
        .route(
            "/api/v1/pricing/estimate",
            post(handlers::pricing::estimate),
        )
        .route("/api/v1/rating/quote", post(handlers::rating::quote))
        .route(
            "/api/v1/quotes/prospect",
            post(handlers::prospect::prospect_quote),
        )
        .route(
            "/api/v1/quotes",
            get(handlers::quotes::list_quotes).post(handlers::quotes::create_quote),
        )
        .route("/api/v1/quotes/:id", get(handlers::quotes::get_quote))
        .route(
            "/api/v1/quotes/:id/timeline",
            get(handlers::quotes::get_quote_timeline),
        )
        .route(
            "/api/v1/quotes/:id/bind",
            post(handlers::quotes::bind_quote),
        )
        .route(
            "/api/v1/quotes/:id/issue",
            post(handlers::policies::issue_policy),
        )
        .route("/api/v1/policies", get(handlers::policies::list_policies))
        .route("/api/v1/policies/:id", get(handlers::policies::get_policy))
        .route("/api/v1/connect/chat", post(handlers::connect::chat))
        .route("/api/v1/connect/memory", get(handlers::connect::memory))
        .route("/api/v1/connect/history", get(handlers::connect::history))
        .route("/api/v1/fnol/submissions", get(handlers::fnol::list_submissions))
        .route("/api/v1/fnol/submit", post(handlers::fnol::submit))
        .route("/api/v1/fnol/:id/trace", get(handlers::fnol::trace))
        .route("/api/v1/fnol/:id/document", get(handlers::fnol::document))
        .route("/api/v1/uw/jobs", get(handlers::uw::list_jobs))
        .route("/api/v1/uw/upload", post(handlers::uw::upload))
        .route("/api/v1/uw/:id/trace", get(handlers::uw::trace))
        .route("/api/v1/uw/:id/document", get(handlers::uw::document))
        .route(
            "/api/v1/hubspot/context/:portal_id/:object_type/:object_id",
            get(handlers::hubspot::get_context).put(handlers::hubspot::upsert_context),
        )
        .route(
            "/api/v1/hubspot/sync/:portal_id/:object_type/:object_id",
            get(handlers::hubspot::get_sync_status),
        )
        .route(
            "/api/v1/hubspot/sync/:portal_id/:object_type/:object_id/retry",
            post(handlers::hubspot::retry_sync),
        )
        .route(
            "/api/v1/admin/hubspot/backfill-links",
            post(handlers::hubspot::backfill_hubspot_links),
        )
        .route(
            "/api/v1/policies/:id/versions",
            get(handlers::policies::get_policy_versions),
        )
        .route(
            "/api/v1/policies/:id/as-of",
            get(handlers::policies::get_policy_as_of),
        )
        .route(
            "/api/v1/policies/:id/document",
            get(handlers::policies::get_policy_document),
        )
        .route(
            "/api/v1/policies/:id/payments",
            get(handlers::policy_workspace::list_payments)
                .post(handlers::policy_workspace::create_payment),
        )
        .route(
            "/api/v1/policies/:id/claims",
            get(handlers::policy_workspace::list_claims)
                .post(handlers::policy_workspace::create_claim),
        )
        .route(
            "/api/v1/policies/:id/claims/:claim_id/reserve",
            get(handlers::policy_workspace::get_claim_reserve_as_of)
                .patch(handlers::policy_workspace::reestimate_claim_reserve),
        )
        .route(
            "/api/v1/claims/:id/fraud-risk",
            get(handlers::fraud::get_fraud_risk)
                .post(handlers::fraud::compute_fraud_risk)
                .patch(handlers::fraud::update_fraud_status),
        )
        .route(
            "/api/v1/customers/:id/kyc",
            get(handlers::kyc::get_kyc_profile)
                .post(handlers::kyc::submit_kyc_profile)
                .patch(handlers::kyc::verify_kyc_profile),
        )
        .route(
            "/api/v1/commissions",
            get(handlers::commissions::list_commissions)
                .post(handlers::commissions::create_commission),
        )
        .route(
            "/api/v1/commissions/:id/status",
            axum::routing::patch(handlers::commissions::update_commission_status),
        )
        .route(
            "/api/v1/policies/:id/notifications",
            get(handlers::policy_workspace::list_notifications),
        )
        .route(
            "/api/v1/policies/:id/transactions",
            get(handlers::policy_workspace::list_transactions),
        )
        .route(
            "/api/v1/policies/:id/renewals",
            get(handlers::policy_workspace::list_renewals)
                .post(handlers::policy_workspace::create_renewal),
        )
        .route(
            "/api/v1/pas/endorse",
            post(handlers::policies::endorse_policy).route_layer(
                axum_middleware::from_fn_with_state(
                    app_state.clone(),
                    policy_lock::require_policy_lock::<AppState>,
                ),
            ),
        )
        .route(
            "/api/v1/pas/cancel",
            post(handlers::policies::cancel_policy).route_layer(
                axum_middleware::from_fn_with_state(
                    app_state.clone(),
                    policy_lock::require_policy_lock::<AppState>,
                ),
            ),
        )
        .route(
            "/api/v1/pas/reinstate",
            post(handlers::policies::reinstate_policy).route_layer(
                axum_middleware::from_fn_with_state(
                    app_state.clone(),
                    policy_lock::require_policy_lock::<AppState>,
                ),
            ),
        )
        .route(
            "/api/v1/pas/oos-endorse",
            post(handlers::oos::oos_endorse).route_layer(axum_middleware::from_fn_with_state(
                app_state.clone(),
                policy_lock::require_policy_lock::<AppState>,
            )),
        )
        .route(
            "/api/v1/agents",
            get(handlers::agents::list_agents).post(handlers::agents::create_agent),
        )
        .route("/api/v1/dashboard/stats", get(handlers::dashboard::stats))
        .route("/api/v1/health/claims", get(handlers::dashboard::claims_health))
        .route("/api/v1/health/policies", get(handlers::dashboard::policies_health))
        .route(
            "/api/v1/notify/whatsapp/send",
            post(handlers::notify::whatsapp_send),
        )
        .route(
            "/api/v1/notify/whatsapp/send-flow",
            post(handlers::notify::whatsapp_send_flow),
        )
        .route(
            "/api/v1/notify/whatsapp/webhook",
            get(handlers::notify::whatsapp_webhook_verify)
                .post(handlers::notify::whatsapp_webhook_receive),
        )
        .layer(axum_middleware::from_fn_with_state(
            app_state.clone(),
            middleware::require_auth,
        ));

    let internal_hubspot_routes = Router::new().route(
        "/api/v1/internal/hubspot/cache/:portal_id/:object_type/:object_id",
        post(handlers::hubspot::cache_inbound_properties).route_layer(
            axum_middleware::from_fn_with_state(
                app_state.clone(),
                handlers::hubspot::require_bridge_secret,
            ),
        ),
    );

    let app = Router::new()
        .merge(authenticated_routes)
        .merge(internal_hubspot_routes)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn resolve_hubspot_bridge_secret(
    sync_secret: Option<String>,
    legacy_bridge_secret: Option<String>,
) -> Option<String> {
    let normalize = |value: String| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    };

    sync_secret
        .and_then(&normalize)
        .or_else(|| legacy_bridge_secret.and_then(normalize))
}

#[cfg(test)]
mod tests {
    use super::resolve_hubspot_bridge_secret;

    #[test]
    fn hubspot_sync_secret_takes_precedence() {
        let secret = resolve_hubspot_bridge_secret(
            Some(" sync-secret ".to_string()),
            Some("legacy-secret".to_string()),
        );
        assert_eq!(secret.as_deref(), Some("sync-secret"));
    }

    #[test]
    fn legacy_hubspot_bridge_secret_remains_a_fallback() {
        let secret = resolve_hubspot_bridge_secret(None, Some(" legacy-secret ".to_string()));
        assert_eq!(secret.as_deref(), Some("legacy-secret"));

        let blank_primary = resolve_hubspot_bridge_secret(
            Some("   ".to_string()),
            Some("legacy-secret".to_string()),
        );
        assert_eq!(blank_primary.as_deref(), Some("legacy-secret"));
    }
}

async fn health(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&**state.db)
        .await
        .is_ok();

    if !db_ok {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "database ping failed".to_string(),
        ));
    }

    let timeline_projection_count = state
        .timeline_projector
        .lock()
        .map(|p| p.timeline_count())
        .unwrap_or(0);

    Ok(Json(serde_json::json!({
        "status": "ok",
        "service": "sagepas-api",
        "time": Utc::now(),
        "database": "connected",
        "eventing": {
            "event_store_backend": state.event_store_backend,
            "projector_bootstrap_event_count": state.projector_bootstrap_event_count,
            "projector_bootstrap_at": state.projector_bootstrap_at,
            "timeline_projection_count": timeline_projection_count
        }
    })))
}

struct BootstrapStats {
    event_count: usize,
    replayed_at: Option<String>,
}

async fn bootstrap_timeline_projector(
    event_store: &Arc<dyn EventStore + Send + Sync>,
    timeline_projector: &Arc<Mutex<EventProjector>>,
) -> anyhow::Result<BootstrapStats> {
    let events = event_store.load_all().await?;
    if events.is_empty() {
        info!("timeline projector bootstrap: no events to replay");
        return Ok(BootstrapStats {
            event_count: 0,
            replayed_at: None,
        });
    }

    let mut projector = timeline_projector
        .lock()
        .map_err(|_| anyhow::anyhow!("timeline projector lock poisoned during bootstrap"))?;

    for event in &events {
        projector.apply(event);
    }

    let replayed_at = Utc::now().to_rfc3339();
    info!(event_count = events.len(), replayed_at = %replayed_at, "timeline projector bootstrap complete");
    Ok(BootstrapStats {
        event_count: events.len(),
        replayed_at: Some(replayed_at),
    })
}
