use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum InsuranceEvent {
    QuoteCreated(QuoteCreated),
    PolicyBound(PolicyBound),
    PolicyIssued(PolicyIssued),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCreated {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub quote_id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub premium: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBound {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub quote_id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyIssued {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub policy_id: Uuid,
    pub quote_id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub premium: f64,
    pub currency: String,
}

impl InsuranceEvent {
    pub fn stream_name(&self) -> String {
        match self {
            Self::QuoteCreated(e) => format!("quote-{}", e.quote_id),
            Self::PolicyBound(e) => format!("quote-{}", e.quote_id),
            Self::PolicyIssued(e) => format!("policy-{}", e.policy_id),
        }
    }
}
