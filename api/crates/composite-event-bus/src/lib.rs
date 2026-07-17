use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use event_bus::EventBus;
use event_models::InsuranceEvent;
use event_router::{EventRoutePolicy, RouteTarget};
use event_store::EventStore;

#[derive(Clone)]
pub struct CompositeEventBus {
    router: Arc<dyn EventRoutePolicy>,
    store: Arc<dyn EventStore + Send + Sync>,
}

impl CompositeEventBus {
    pub fn new(
        router: Arc<dyn EventRoutePolicy>,
        store: Arc<dyn EventStore + Send + Sync>,
    ) -> Self {
        Self { router, store }
    }
}

#[async_trait]
impl EventBus for CompositeEventBus {
    async fn publish(&self, event: InsuranceEvent) -> Result<()> {
        if self.router.route(&event).contains(&RouteTarget::EventStore) {
            self.store.append(&event.stream_name(), event).await?;
        }
        Ok(())
    }
}
