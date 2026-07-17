use event_models::InsuranceEvent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteTarget {
    EventStore,
}

pub trait EventRoutePolicy: Send + Sync {
    fn route(&self, event: &InsuranceEvent) -> Vec<RouteTarget>;
}

/// Every retained SagePAS lifecycle event is durably written to PostgreSQL.
pub struct DefaultRoutePolicy;

impl EventRoutePolicy for DefaultRoutePolicy {
    fn route(&self, _event: &InsuranceEvent) -> Vec<RouteTarget> {
        vec![RouteTarget::EventStore]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use event_models::QuoteCreated;
    use uuid::Uuid;

    #[test]
    fn routes_quote_created_to_event_store() {
        let event = InsuranceEvent::QuoteCreated(QuoteCreated {
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            quote_id: Uuid::new_v4(),
            customer_id: Uuid::new_v4(),
            product_id: Uuid::new_v4(),
            premium: 123.45,
            currency: "USD".to_string(),
        });
        assert_eq!(
            DefaultRoutePolicy.route(&event),
            vec![RouteTarget::EventStore]
        );
    }
}
