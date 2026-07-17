use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProductRow {
    pub id: Uuid,
    pub name: String,
    pub insurance_type: String,
    pub country: String,
    pub currency: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

/// GET /api/v1/products
pub async fn list_products(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProductRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, ProductRow>(
        r#"
        SELECT id, name, insurance_type, country, currency, description, created_at
        FROM products
        ORDER BY name ASC
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}
