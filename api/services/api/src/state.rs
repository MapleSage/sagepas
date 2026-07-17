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
    /// Microsoft Entra token validator — `None` when `AZURE_ENTRA_TENANT_ID`
    /// / `AZURE_ENTRA_AUDIENCE` are not configured.
    pub entra_validator: Option<Arc<infra::entra_auth::EntraValidator>>,
}

impl HasPolicyLock for AppState {
    fn policy_lock(&self) -> Arc<PolicyLockManager> {
        self.policy_lock.clone()
    }
}
