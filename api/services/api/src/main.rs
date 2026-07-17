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
mod handlers;
mod middleware;
mod migration_runner;
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

    let blob = Arc::new(BlobClient::from_managed_identity(
        &config.storage_account_name,
        &config.storage_container_name,
    )?);
    let documents = Arc::new(DocumentGenerator::new(
        (*blob).clone(),
        config.storage_container_name.clone(),
        &config.storage_account_name,
    ));

    let entra_validator = if config.entra_configured() {
        info!(tenant = %config.azure_entra_tenant_id, "Entra token validation enabled");
        Some(Arc::new(infra::entra_auth::EntraValidator::new(
            config.azure_entra_tenant_id.clone(),
            config.azure_entra_audience.clone(),
            if config.azure_entra_issuer.trim().is_empty() {
                None
            } else {
                Some(config.azure_entra_issuer.clone())
            },
        )))
    } else {
        warn!(
            "AZURE_ENTRA_TENANT_ID / AZURE_ENTRA_AUDIENCE not configured; Entra token validation disabled"
        );
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
        entra_validator,
    };

    let app = Router::new()
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
        .layer(axum_middleware::from_fn_with_state(
            app_state.clone(),
            middleware::require_auth,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "listening");
    axum::serve(listener, app).await?;
    Ok(())
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
