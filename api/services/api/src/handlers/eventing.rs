use crate::state::AppState;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct EventingStatusResponse {
    pub event_store_backend: String,
    pub projector_bootstrap_event_count: u64,
    pub projector_bootstrap_at: Option<String>,
    pub timeline_projection_count: usize,
    pub total_events: i64,
    pub latest_sequence: Option<i64>,
    pub latest_at: Option<DateTime<Utc>>,
    pub store: String,
}

pub async fn status(State(state): State<AppState>) -> Json<EventingStatusResponse> {
    let timeline_projection_count = state
        .timeline_projector
        .lock()
        .map(|p| p.timeline_count())
        .unwrap_or(0);

    let event_stats = sqlx::query_as::<_, (i64, Option<i64>, Option<DateTime<Utc>>)>(
        "SELECT COUNT(*)::BIGINT, MAX(sequence), MAX(inserted_at) FROM postgres_events",
    )
    .fetch_one(&**state.db)
    .await
    .unwrap_or((0, None, None));

    Json(EventingStatusResponse {
        event_store_backend: state.event_store_backend.clone(),
        projector_bootstrap_event_count: state.projector_bootstrap_event_count,
        projector_bootstrap_at: state.projector_bootstrap_at.clone(),
        timeline_projection_count,
        total_events: event_stats.0,
        latest_sequence: event_stats.1,
        latest_at: event_stats.2,
        store: state.event_store_backend.clone(),
    })
}
