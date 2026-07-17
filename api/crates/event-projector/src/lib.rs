use std::collections::HashMap;

use chrono::{DateTime, Utc};
use event_models::{InsuranceEvent, PolicyBound, PolicyIssued, QuoteCreated};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    QuoteCreated,
    PolicyBound,
    PolicyIssued,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub stage: LifecycleStage,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyTimelineProjection {
    pub quote_id: Uuid,
    pub policy_id: Option<Uuid>,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub premium: Option<f64>,
    pub currency: Option<String>,
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Default, Clone)]
pub struct EventProjector {
    by_quote: HashMap<Uuid, PolicyTimelineProjection>,
    policy_to_quote: HashMap<Uuid, Uuid>,
}

impl EventProjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &InsuranceEvent) {
        match event {
            InsuranceEvent::QuoteCreated(e) => self.apply_quote_created(e),
            InsuranceEvent::PolicyBound(e) => self.apply_policy_bound(e),
            InsuranceEvent::PolicyIssued(e) => self.apply_policy_issued(e),
        }
    }

    pub fn apply_all<'a, I>(&mut self, events: I)
    where
        I: IntoIterator<Item = &'a InsuranceEvent>,
    {
        for event in events {
            self.apply(event);
        }
    }

    pub fn timeline_by_quote(&self, quote_id: Uuid) -> Option<&PolicyTimelineProjection> {
        self.by_quote.get(&quote_id)
    }

    pub fn timeline_by_policy(&self, policy_id: Uuid) -> Option<&PolicyTimelineProjection> {
        let quote_id = self.policy_to_quote.get(&policy_id)?;
        self.by_quote.get(quote_id)
    }

    pub fn timeline_count(&self) -> usize {
        self.by_quote.len()
    }

    fn apply_quote_created(&mut self, e: &QuoteCreated) {
        let timeline =
            self.by_quote
                .entry(e.quote_id)
                .or_insert_with(|| PolicyTimelineProjection {
                    quote_id: e.quote_id,
                    policy_id: None,
                    customer_id: e.customer_id,
                    product_id: e.product_id,
                    premium: Some(e.premium),
                    currency: Some(e.currency.clone()),
                    entries: Vec::new(),
                });

        timeline.customer_id = e.customer_id;
        timeline.product_id = e.product_id;
        timeline.premium = Some(e.premium);
        timeline.currency = Some(e.currency.clone());
        push_unique_stage(
            &mut timeline.entries,
            LifecycleStage::QuoteCreated,
            e.occurred_at,
        );
    }

    fn apply_policy_bound(&mut self, e: &PolicyBound) {
        let timeline =
            self.by_quote
                .entry(e.quote_id)
                .or_insert_with(|| PolicyTimelineProjection {
                    quote_id: e.quote_id,
                    policy_id: None,
                    customer_id: e.customer_id,
                    product_id: e.product_id,
                    premium: None,
                    currency: None,
                    entries: Vec::new(),
                });

        timeline.customer_id = e.customer_id;
        timeline.product_id = e.product_id;
        push_unique_stage(
            &mut timeline.entries,
            LifecycleStage::PolicyBound,
            e.occurred_at,
        );
    }

    fn apply_policy_issued(&mut self, e: &PolicyIssued) {
        let timeline =
            self.by_quote
                .entry(e.quote_id)
                .or_insert_with(|| PolicyTimelineProjection {
                    quote_id: e.quote_id,
                    policy_id: Some(e.policy_id),
                    customer_id: e.customer_id,
                    product_id: e.product_id,
                    premium: Some(e.premium),
                    currency: Some(e.currency.clone()),
                    entries: Vec::new(),
                });

        timeline.policy_id = Some(e.policy_id);
        timeline.customer_id = e.customer_id;
        timeline.product_id = e.product_id;
        timeline.premium = Some(e.premium);
        timeline.currency = Some(e.currency.clone());
        push_unique_stage(
            &mut timeline.entries,
            LifecycleStage::PolicyIssued,
            e.occurred_at,
        );

        self.policy_to_quote.insert(e.policy_id, e.quote_id);
    }
}

fn push_unique_stage(
    entries: &mut Vec<TimelineEntry>,
    stage: LifecycleStage,
    occurred_at: DateTime<Utc>,
) {
    if entries.iter().any(|entry| entry.stage == stage) {
        return;
    }
    entries.push(TimelineEntry { stage, occurred_at });
    entries.sort_by_key(|entry| entry.occurred_at);
}

#[cfg(test)]
mod tests {
    use super::*;
    use event_models::{PolicyBound, PolicyIssued, QuoteCreated};

    #[test]
    fn builds_timeline_from_quote_bind_issue() {
        let quote_id = Uuid::new_v4();
        let policy_id = Uuid::new_v4();
        let customer_id = Uuid::new_v4();
        let product_id = Uuid::new_v4();

        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::minutes(5);
        let t3 = t2 + chrono::Duration::minutes(5);

        let events = vec![
            InsuranceEvent::QuoteCreated(QuoteCreated {
                event_id: Uuid::new_v4(),
                occurred_at: t1,
                quote_id,
                customer_id,
                product_id,
                premium: 1200.0,
                currency: "USD".to_string(),
            }),
            InsuranceEvent::PolicyBound(PolicyBound {
                event_id: Uuid::new_v4(),
                occurred_at: t2,
                quote_id,
                customer_id,
                product_id,
            }),
            InsuranceEvent::PolicyIssued(PolicyIssued {
                event_id: Uuid::new_v4(),
                occurred_at: t3,
                policy_id,
                quote_id,
                customer_id,
                product_id,
                premium: 1200.0,
                currency: "USD".to_string(),
            }),
        ];

        let mut projector = EventProjector::new();
        projector.apply_all(events.iter());

        let timeline = projector
            .timeline_by_quote(quote_id)
            .expect("timeline by quote");
        assert_eq!(timeline.policy_id, Some(policy_id));
        assert_eq!(timeline.entries.len(), 3);
        assert_eq!(timeline.entries[0].stage, LifecycleStage::QuoteCreated);
        assert_eq!(timeline.entries[1].stage, LifecycleStage::PolicyBound);
        assert_eq!(timeline.entries[2].stage, LifecycleStage::PolicyIssued);

        let by_policy = projector
            .timeline_by_policy(policy_id)
            .expect("timeline by policy");
        assert_eq!(by_policy.quote_id, quote_id);
    }
}
