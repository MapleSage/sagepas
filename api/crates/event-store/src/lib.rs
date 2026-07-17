use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use event_models::InsuranceEvent;
use infra::db::DbPool;
use tokio::sync::RwLock;

#[async_trait]
pub trait EventStore {
    async fn append(&self, stream: &str, event: InsuranceEvent) -> Result<()>;
    async fn load(&self, stream: &str) -> Result<Vec<InsuranceEvent>>;
    async fn load_all(&self) -> Result<Vec<InsuranceEvent>>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryEventStore {
    streams: Arc<RwLock<HashMap<String, Vec<InsuranceEvent>>>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, stream: &str, event: InsuranceEvent) -> Result<()> {
        self.streams
            .write()
            .await
            .entry(stream.to_string())
            .or_default()
            .push(event);
        Ok(())
    }

    async fn load(&self, stream: &str) -> Result<Vec<InsuranceEvent>> {
        Ok(self
            .streams
            .read()
            .await
            .get(stream)
            .cloned()
            .unwrap_or_default())
    }

    async fn load_all(&self) -> Result<Vec<InsuranceEvent>> {
        Ok(self
            .streams
            .read()
            .await
            .values()
            .flat_map(|events| events.clone())
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct PostgresEventStore {
    pool: Arc<DbPool>,
}

impl PostgresEventStore {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PostgresEventRow {
    payload: serde_json::Value,
}

#[async_trait]
impl EventStore for PostgresEventStore {
    async fn append(&self, stream: &str, event: InsuranceEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO postgres_events (stream_name, event_type, payload, occurred_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(stream)
        .bind(event_type(&event))
        .bind(serde_json::to_value(&event)?)
        .bind(occurred_at(&event))
        .execute(&**self.pool)
        .await?;
        Ok(())
    }

    async fn load(&self, stream: &str) -> Result<Vec<InsuranceEvent>> {
        let rows = sqlx::query_as::<_, PostgresEventRow>(
            "SELECT payload FROM postgres_events WHERE stream_name = $1 ORDER BY sequence ASC",
        )
        .bind(stream)
        .fetch_all(&**self.pool)
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_value(row.payload).map_err(Into::into))
            .collect()
    }

    async fn load_all(&self) -> Result<Vec<InsuranceEvent>> {
        let rows = sqlx::query_as::<_, PostgresEventRow>(
            "SELECT payload FROM postgres_events ORDER BY sequence ASC",
        )
        .fetch_all(&**self.pool)
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_value(row.payload).map_err(Into::into))
            .collect()
    }
}

fn event_type(event: &InsuranceEvent) -> &'static str {
    match event {
        InsuranceEvent::QuoteCreated(_) => "QuoteCreated",
        InsuranceEvent::PolicyBound(_) => "PolicyBound",
        InsuranceEvent::PolicyIssued(_) => "PolicyIssued",
    }
}

fn occurred_at(event: &InsuranceEvent) -> DateTime<Utc> {
    match event {
        InsuranceEvent::QuoteCreated(e) => e.occurred_at,
        InsuranceEvent::PolicyBound(e) => e.occurred_at,
        InsuranceEvent::PolicyIssued(e) => e.occurred_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use event_models::QuoteCreated;
    use uuid::Uuid;

    fn quote_event(quote_id: Uuid) -> InsuranceEvent {
        InsuranceEvent::QuoteCreated(QuoteCreated {
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            quote_id,
            customer_id: Uuid::new_v4(),
            product_id: Uuid::new_v4(),
            premium: 123.45,
            currency: "USD".to_string(),
        })
    }

    #[tokio::test]
    async fn in_memory_store_filters_streams_and_loads_all() {
        let store = InMemoryEventStore::new();
        let quote_a = Uuid::new_v4();
        let quote_b = Uuid::new_v4();
        store
            .append(&format!("quote-{quote_a}"), quote_event(quote_a))
            .await
            .unwrap();
        store
            .append(&format!("quote-{quote_b}"), quote_event(quote_b))
            .await
            .unwrap();

        assert_eq!(
            store.load(&format!("quote-{quote_a}")).await.unwrap().len(),
            1
        );
        assert_eq!(store.load_all().await.unwrap().len(), 2);
    }
}
