use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;
use domain::auth::PasRole;

#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: String,
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub national_id: Option<String>,
    pub national_id_type: Option<String>,
    pub address: Option<String>,
}

fn default_country() -> String {
    "US".into()
}
fn default_currency() -> String {
    "USD".into()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CustomerRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub country: String,
    pub currency: String,
    pub national_id: Option<String>,
    pub national_id_type: Option<String>,
    pub address: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

/// POST /api/v1/customers
pub async fn create_customer(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateCustomerRequest>,
) -> Result<Json<CustomerRow>, (StatusCode, String)> {
    require_roles(
        &user,
        &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter],
    )?;
    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, CustomerRow>(
        r#"
        INSERT INTO customers
            (id, user_id, name, email, phone, country, currency,
             national_id, national_id_type, address)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, user_id, name, email, phone, country, currency,
                  national_id, national_id_type, address, created_at
        "#,
    )
    .bind(id)
    .bind(req.user_id)
    .bind(req.name)
    .bind(req.email)
    .bind(req.phone)
    .bind(req.country)
    .bind(req.currency)
    .bind(req.national_id)
    .bind(req.national_id_type)
    .bind(req.address)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(row))
}

/// GET /api/v1/customers
pub async fn list_customers(
    State(state): State<AppState>,
) -> Result<Json<Vec<CustomerRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, CustomerRow>(
        r#"
        SELECT id, user_id, name, email, phone, country, currency,
               national_id, national_id_type, address, created_at
        FROM customers
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}

/// GET /api/v1/customers/:id
pub async fn get_customer(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<CustomerRow>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, CustomerRow>(
        r#"
        SELECT id, user_id, name, email, phone, country, currency,
               national_id, national_id_type, address, created_at
        FROM customers
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "customer not found".to_string()))?;

    Ok(Json(row))
}
