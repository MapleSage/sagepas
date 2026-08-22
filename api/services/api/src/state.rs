use std::sync::{Arc, Mutex};

use documents::DocumentGenerator;
use event_bus::EventBus;
use event_projector::EventProjector;
use event_store::EventStore;
use infra::{config::AppConfig, db::DbPool};
use oos_orchestrator::OosOrchestrator;

use policy_ledger::{BiTemporalPolicyStore, PolicyLedger};
use policy_lock::{HasPolicyLock, PolicyLockManager};
use pricing::PricingEngine;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DbPool>,
    pub config: Arc<AppConfig>,
    pub pricing: Arc<PricingEngine>,
    pub documents: Arc<DocumentGenerator>,
    pub ledger: Arc<PolicyLedger>,
    pub bitemporal_policy: Arc<BiTemporalPolicyStore>,
    pub oos_orchestrator: Arc<OosOrchestrator>,
    pub policy_lock: Arc<PolicyLockManager>,
    pub event_bus: Arc<dyn EventBus + Send + Sync>,
    pub event_store: Arc<dyn EventStore + Send + Sync>,
    pub timeline_projector: Arc<Mutex<EventProjector>>,
    pub event_store_backend: String,
    pub projector_bootstrap_event_count: u64,
    pub projector_bootstrap_at: Option<String>,
    /// Full internal bridge ingestion URL. This is integration configuration,
    /// never a HubSpot credential and never returned from an API response.
    pub hubspot_bridge_url: Option<String>,
    /// Dedicated shared secret for the internal bridge boundary. It is held in
    /// process memory only and is never persisted or serialized.
    pub hubspot_bridge_secret: Option<String>,
    /// Portal ID written into `hubspot_record_links` and validated against
    /// by the bridge's own `identity()` check. Configurable (`HUBSPOT_PORTAL_ID`)
    /// so a promotion to a different HubSpot portal is a config change, not
    /// a code change -- work order item 9's Rust-side twin of the same
    /// hardcoded-portal problem fixed in SagePasSync.js.
    pub hubspot_portal_id: i64,
    pub hubspot_http_client: reqwest::Client,
    /// Wakes the durable dispatcher after a transaction commits new work.
    pub hubspot_dispatch_notify: Arc<tokio::sync::Notify>,
    /// Microsoft Entra token validator — `None` when `AZURE_ENTRA_TENANT_ID`
    /// / `AZURE_ENTRA_AUDIENCE` are not configured.
    pub entra_validator: Option<Arc<infra::entra_auth::EntraValidator>>,
    /// Guards the unauthenticated prospect-quote endpoint (work order item 2).
    pub prospect_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Double-entry bitemporal subledger (accounting_batches/journal_entries/
    /// journal_lines, migration 006). Existed unwired since Phase 2 until
    /// work order item 4 made claims case-reserve posting its first caller.
    /// Cheap to clone: wraps a single `Arc<DbPool>` internally.
    pub subledger: premium_ledger::PremiumSubledger,
}

impl HasPolicyLock for AppState {
    fn policy_lock(&self) -> Arc<PolicyLockManager> {
        self.policy_lock.clone()
    }
}
