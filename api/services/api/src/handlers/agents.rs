use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;
use domain::auth::PasRole;

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub email: String,
    #[serde(default = "default_commission")]
    pub commission_rate_pct: f64,
    #[serde(default = "default_country")]
    pub country: String,
}

fn default_commission() -> f64 {
    5.0
}
fn default_country() -> String {
    "US".into()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AgentRow {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub commission_rate_pct: f64,
    pub country: String,
    pub created_at: chrono::DateTime<Utc>,
}

/// POST /api/v1/agents
pub async fn create_agent(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<AgentRow>, (StatusCode, String)> {
    require_roles(&user, &[PasRole::Admin])?;
    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, AgentRow>(
        r#"
        INSERT INTO agents (id, name, email, commission_rate_pct, country)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name, email, commission_rate_pct, country, created_at
        "#,
    )
    .bind(id)
    .bind(req.name)
    .bind(req.email)
    .bind(req.commission_rate_pct)
    .bind(req.country)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(row))
}

/// GET /api/v1/agents
pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, AgentRow>(
        r#"
        SELECT id, name, email, commission_rate_pct, country, created_at
        FROM agents
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}
