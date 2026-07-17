use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use event_models::InsuranceEvent;
use tokio::sync::RwLock;

#[async_trait]
pub trait EventBus {
    async fn publish(&self, event: InsuranceEvent) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryEventBus {
    events: Arc<RwLock<Vec<InsuranceEvent>>>,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn all(&self) -> Vec<InsuranceEvent> {
        self.events.read().await.clone()
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, event: InsuranceEvent) -> Result<()> {
        self.events.write().await.push(event);
        Ok(())
    }
}
