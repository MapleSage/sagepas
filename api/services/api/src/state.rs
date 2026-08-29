use std::sync::{Arc, Mutex};

use documents::DocumentGenerator;
use event_bus::EventBus;
use event_projector::EventProjector;
use event_store::EventStore;
use infra::{config::AppConfig, db::DbPool, search::SearchClient};
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
    /// Microsoft Entra token validator, staff/Workforce tenant — `None` when
    /// `AZURE_ENTRA_TENANT_ID`/`AZURE_ENTRA_AUDIENCE` are not configured.
    pub entra_staff_validator: Option<Arc<infra::entra_auth::EntraValidator>>,
    /// Entra External ID (CIAM) validator, consumer/policyholder self-service
    /// (work order Phase 8) — `None` when `AZURE_ENTRA_CONSUMER_TENANT_ID`/
    /// `AZURE_ENTRA_CONSUMER_AUDIENCE` are not configured. `middleware::
    /// try_validate` tries staff then consumer; each independently rejects
    /// on a mismatched `tid`, so trying both is cheap and safe.
    pub entra_consumer_validator: Option<Arc<infra::entra_auth::EntraValidator>>,
    /// Validates sesure-us's own staff audience, accepted ONLY on the
    /// explicit delegated-read path allowlist and only combined with a real
    /// STAFF_ROLES claim (work order Phase 9) -- see `AppConfig::
    /// azure_entra_delegate_audience`'s doc comment for why this isn't a
    /// blanket audience add.
    pub entra_delegate_validator: Option<Arc<infra::entra_auth::EntraValidator>>,
    /// Guards the unauthenticated prospect-quote endpoint (work order item 2).
    pub prospect_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Double-entry bitemporal subledger (accounting_batches/journal_entries/
    /// journal_lines, migration 006). Existed unwired since Phase 2 until
    /// work order item 4 made claims case-reserve posting its first caller.
    /// Cheap to clone: wraps a single `Arc<DbPool>` internally.
    pub subledger: premium_ledger::PremiumSubledger,
    /// Content Understanding client for the shared FNOL/UW pipeline
    /// (`doc-pipeline`). `None` when `CONTENT_UNDERSTANDING_ENDPOINT` is
    /// unconfigured -- the pipeline degrades to a GPT-only path rather than
    /// failing closed.
    pub cu_client: Option<Arc<doc_pipeline::content_understanding::ContentUnderstandingClient>>,
    pub openai_client: Arc<infra::openai::OpenAIClient>,
    /// Blob containers are separate per FNOL consolidation decision Phase 3
    /// (`fnol-documents` / `uw-documents`, already named in migrations
    /// 009-011) -- two clients, not one, since `BlobClient` is
    /// container-scoped at construction.
    pub fnol_blob: Arc<infra::blob::BlobClient>,
    pub uw_blob: Arc<infra::blob::BlobClient>,
    /// Azure AI Search -- KB-grounded scoring for FNOL/UW (`doc-pipeline`'s
    /// `kb_scoring` module) and PAS pricing factors both read this.
    pub search: Arc<SearchClient>,
    /// GIA's chat history + fact memory (`handlers/connect.rs`). sagepas
    /// had no equivalent of this at all before -- ported from sagesure-us.
    pub conversation: conversation_memory::ConversationStore,
    /// Native Meta WhatsApp Cloud API sender, ported from sagesure-us's
    /// `whatsapp` crate. `None` when `WHATSAPP_ACCESS_TOKEN`/`WHATSAPP_PHONE_ID`
    /// are unconfigured -- unlike sesure-us there is no separate Python
    /// notifications service to fall back to.
    pub whatsapp_client: Option<Arc<whatsapp::WhatsAppClient>>,
}

impl HasPolicyLock for AppState {
    fn policy_lock(&self) -> Arc<PolicyLockManager> {
        self.policy_lock.clone()
    }
}
