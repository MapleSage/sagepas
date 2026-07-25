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
#[serde(deny_unknown_fields)]
pub struct CreateCustomerRequest {
    pub name: String,
    pub email: String,
    pub phone: String,
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default)]
    pub national_id: Option<String>,
    #[serde(default)]
    pub national_id_type: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}

fn default_country() -> String {
    "US".into()
}
fn default_currency() -> String {
    "USD".into()
}

fn validate_customer(
    mut req: CreateCustomerRequest,
) -> Result<CreateCustomerRequest, (StatusCode, String)> {
    req.name = req.name.trim().to_string();
    req.email = req.email.trim().to_lowercase();
    req.phone = req.phone.trim().to_string();
    req.country = req.country.trim().to_uppercase();
    req.currency = req.currency.trim().to_uppercase();
    req.national_id = req
        .national_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    req.national_id_type = req
        .national_id_type
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    req.address = req
        .address
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let email_parts: Vec<_> = req.email.split('@').collect();
    let valid_email = email_parts.len() == 2
        && !email_parts[0].is_empty()
        && !email_parts[1].is_empty()
        && !req.email.chars().any(char::is_whitespace);
    if req.name.is_empty() || req.name.len() > 200 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "customer name must be between 1 and 200 characters".into(),
        ));
    }
    if !valid_email || req.email.len() > 254 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "customer email is invalid".into(),
        ));
    }
    if req.phone.is_empty() || req.phone.len() > 50 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "customer phone must be between 1 and 50 characters".into(),
        ));
    }
    if req.country.len() != 2 || !req.country.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "country must be a two-letter code".into(),
        ));
    }
    if req.currency.len() != 3 || !req.currency.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "currency must be a three-letter code".into(),
        ));
    }
    if req.address.as_ref().is_some_and(|value| value.len() > 2000) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "customer address must not exceed 2000 characters".into(),
        ));
    }

    Ok(req)
}

fn customer_write_error(error: sqlx::Error) -> (StatusCode, String) {
    if let sqlx::Error::Database(database_error) = &error {
        return match database_error.code().as_deref() {
            Some("23505") => (StatusCode::CONFLICT, "customer already exists".into()),
            Some("23502" | "23503" | "23514" | "22001") => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "customer data is invalid".into(),
            ),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
    }

    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
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
    let req = validate_customer(req)?;
    let mut tx = state.db.begin().await.map_err(customer_write_error)?;

    // The schema has no customer-email uniqueness constraint. Serialize writes for the
    // normalized email so the duplicate check and inserts remain atomic without a migration.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&req.email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(customer_write_error)?;

    let duplicate = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM customers WHERE LOWER(email) = $1)",
    )
    .bind(&req.email)
    .fetch_one(&mut *tx)
    .await
    .map_err(customer_write_error)?;
    if duplicate {
        return Err((
            StatusCode::CONFLICT,
            "a customer with this email already exists".into(),
        ));
    }

    let user_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE LOWER(email) = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(&req.email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(customer_write_error)?;
    let user_id = match user_id {
        Some(user_id) => user_id,
        None => {
            let user_id = Uuid::new_v4();
            let password_hash = format!("customer-only:{user_id}");
            sqlx::query_scalar::<_, Uuid>(
                r#"
                INSERT INTO users (id, email, name, password_hash, role)
                VALUES ($1, $2, $3, $4, 'consumer')
                ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email
                RETURNING id
                "#,
            )
            .bind(user_id)
            .bind(&req.email)
            .bind(&req.name)
            .bind(password_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(customer_write_error)?
        }
    };

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
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(req.name)
    .bind(req.email)
    .bind(req.phone)
    .bind(req.country)
    .bind(req.currency)
    .bind(req.national_id)
    .bind(req.national_id_type)
    .bind(req.address)
    .fetch_one(&mut *tx)
    .await
    .map_err(customer_write_error)?;

    tx.commit().await.map_err(customer_write_error)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_does_not_require_user_id() {
        let req: CreateCustomerRequest = serde_json::from_value(serde_json::json!({
            "name": "  Jane Doe  ",
            "email": " Jane.Doe@Example.com ",
            "phone": " +1 555 0100 "
        }))
        .expect("customer payload should deserialize without user_id");

        let req = validate_customer(req).expect("customer payload should validate");
        assert_eq!(req.name, "Jane Doe");
        assert_eq!(req.email, "jane.doe@example.com");
        assert_eq!(req.phone, "+1 555 0100");
        assert_eq!(req.country, "US");
        assert_eq!(req.currency, "USD");
    }

    #[test]
    fn create_request_rejects_client_user_identity_and_role() {
        for untrusted_field in ["user_id", "role"] {
            let mut payload = serde_json::json!({
                "name": "Jane Doe",
                "email": "jane@example.com",
                "phone": "+1 555 0100"
            });
            payload[untrusted_field] = serde_json::json!(Uuid::new_v4().to_string());

            assert!(serde_json::from_value::<CreateCustomerRequest>(payload).is_err());
        }
    }

    #[test]
    fn invalid_customer_data_returns_unprocessable_entity() {
        let req: CreateCustomerRequest = serde_json::from_value(serde_json::json!({
            "name": "Jane Doe",
            "email": "not-an-email",
            "phone": "+1 555 0100"
        }))
        .unwrap();

        let error = validate_customer(req).unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.1, "customer email is invalid");
    }
}
