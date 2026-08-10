use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, NaiveDate, Utc};
use event_models::{InsuranceEvent, PolicyIssued};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::{error, warn};
use uuid::Uuid;

use crate::auth_extract::{AuthUser, require_roles};
use crate::state::AppState;
use domain::auth::PasRole;
use domain::ids::{CustomerId, PolicyId, ProductId, QuoteId};
use domain::insurance::{Currency, Customer, Policy, PolicyState, Product};
use pas_domain::CoverageSelection;
use policy_ledger::PolicyVersionInput;

/// Policy lifecycle mutations (endorse/cancel/reinstate) are administrative
/// actions — a customer's own validated identity is not sufficient.
const LIFECYCLE_MUTATION_ROLES: &[PasRole] =
    &[PasRole::Admin, PasRole::Agent, PasRole::Underwriter];

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PolicyRow {
    pub id: Uuid,
    pub quote_id: Uuid,
    pub policy_number: String,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub state: String,
    pub premium: f64,
    pub currency: String,
    pub start_date: chrono::DateTime<Utc>,
    pub end_date: chrono::DateTime<Utc>,
    pub pdf_url: Option<String>,
    pub coverage: serde_json::Value,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct EndorsePolicyRequest {
    pub policy_id: Uuid,
    pub coverages: Vec<CoverageSelection>,
}

#[derive(Debug, Deserialize)]
pub struct CancelPolicyRequest {
    pub policy_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ReinstatePolicyRequest {
    pub policy_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct QuoteForIssue {
    id: Uuid,
    customer_id: Uuid,
    product_id: Uuid,
    premium: f64,
    currency: String,
    coverage: serde_json::Value,
}

#[derive(Debug, sqlx::FromRow)]
struct CustomerForDoc {
    id: Uuid,
    user_id: Uuid,
    name: String,
    email: String,
    phone: String,
    country: String,
    currency: String,
    national_id: Option<String>,
    national_id_type: Option<String>,
    address: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ProductForDoc {
    id: Uuid,
    name: String,
    insurance_type: String,
    country: String,
    currency: String,
    description: Option<String>,
}

/// GET /api/v1/policies
pub async fn list_policies(
    State(state): State<AppState>,
) -> Result<Json<Vec<PolicyRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, PolicyRow>(
        r#"
        SELECT id, quote_id, policy_number, customer_id, product_id, state,
               premium, currency, start_date, end_date, pdf_url, coverage, created_at
        FROM policies
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .fetch_all(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}

/// GET /api/v1/policies/:id
pub async fn get_policy(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PolicyRow>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, PolicyRow>(
        r#"
        SELECT id, quote_id, policy_number, customer_id, product_id, state,
               premium, currency, start_date, end_date, pdf_url, coverage, created_at
        FROM policies
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".to_string()))?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct PolicyAsOfQuery {
    pub date: NaiveDate,
}

/// GET /api/v1/policies/:id/versions
pub async fn get_policy_versions(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<policy_ledger::PolicyVersion>>, (StatusCode, String)> {
    let versions = state
        .bitemporal_policy
        .full_history(PolicyId(id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(versions))
}

/// GET /api/v1/policies/:id/as-of?date=YYYY-MM-DD
pub async fn get_policy_as_of(
    Path(id): Path<Uuid>,
    Query(query): Query<PolicyAsOfQuery>,
    State(state): State<AppState>,
) -> Result<Json<policy_ledger::PolicyVersion>, (StatusCode, String)> {
    let version = state
        .bitemporal_policy
        .as_of_business_date(PolicyId(id), query.date)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "policy version not found".to_string(),
            )
        })?;

    Ok(Json(version))
}

/// POST /api/v1/quotes/:id/issue
///
/// Creates a policy from a bound quote, generates the PDF declaration page,
/// and marks the quote as `issued`.
pub async fn issue_policy(
    Path(quote_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<PolicyRow>, (StatusCode, String)> {
    require_roles(&user, LIFECYCLE_MUTATION_ROLES)?;
    let quote = sqlx::query_as::<_, QuoteForIssue>(
        r#"
        SELECT id, customer_id, product_id, premium, currency, coverage
        FROM quotes
        WHERE id = $1 AND state = 'bound'
        "#,
    )
    .bind(quote_id)
    .fetch_optional(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            "quote must be in 'bound' state to issue".to_string(),
        )
    })?;

    let customer_row = sqlx::query_as::<_, CustomerForDoc>(
        r#"
        SELECT id, user_id, name, email, phone, country, currency,
               national_id, national_id_type, address, created_at
        FROM customers
        WHERE id = $1
        "#,
    )
    .bind(quote.customer_id)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let product_row = sqlx::query_as::<_, ProductForDoc>(
        r#"SELECT id, name, insurance_type, country, currency, description FROM products WHERE id = $1"#,
    )
    .bind(quote.product_id)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let policy_id = Uuid::new_v4();
    let policy_number = format!(
        "POL-{}",
        &policy_id.to_string().replace('-', "")[..8].to_uppercase()
    );
    let now = Utc::now();
    let start_date = now;
    let end_date = now + Duration::days(365);
    let coverage = quote.coverage;

    let currency = currency_from_code(&quote.currency);
    let policy_domain = Policy {
        id: PolicyId(policy_id),
        quote_id: QuoteId(quote.id),
        policy_number: policy_number.clone(),
        customer_id: CustomerId(customer_row.id),
        product_id: ProductId(product_row.id),
        state: PolicyState::Active,
        premium: quote.premium,
        currency: currency.clone(),
        start_date,
        end_date,
        pdf_url: None,
        coverage: coverage.clone(),
        created_at: now,
    };

    let national_id_type = customer_row.national_id_type.as_deref().and_then(|s| {
        use domain::insurance::NationalIdType;
        match s {
            "ssn" => Some(NationalIdType::Ssn),
            "itin" => Some(NationalIdType::Itin),
            "aadhar" => Some(NationalIdType::Aadhar),
            "pan" => Some(NationalIdType::Pan),
            _ => Some(NationalIdType::Other),
        }
    });

    let customer_domain = Customer {
        id: CustomerId(customer_row.id),
        user_id: domain::ids::UserId(customer_row.user_id),
        name: customer_row.name,
        email: customer_row.email,
        phone: customer_row.phone,
        country: customer_row.country,
        currency: currency_from_code(&customer_row.currency),
        national_id: customer_row.national_id,
        national_id_type,
        address: customer_row.address,
        created_at: customer_row.created_at,
    };

    let product_domain = Product {
        id: ProductId(product_row.id),
        name: product_row.name,
        insurance_type: insurance_type_from_code(&product_row.insurance_type),
        country: product_row.country,
        currency: currency_from_code(&product_row.currency),
        description: product_row.description,
    };

    let pdf_url = if let Ok(local_dir) = std::env::var("LOCAL_DOCUMENT_DIR") {
        let local_dir = local_dir.trim();
        if local_dir.is_empty() {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "LOCAL_DOCUMENT_DIR cannot be empty when configured".to_string(),
            ));
        }
        let bytes = state
            .documents
            .render_policy_document(&policy_domain, &customer_domain, &product_domain)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("pdf rendering: {e:#}"),
                )
            })?;
        tokio::fs::create_dir_all(local_dir).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("document directory: {e}"),
            )
        })?;
        let path = std::path::Path::new(local_dir).join(format!("{policy_number}.pdf"));
        tokio::fs::write(&path, bytes).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("document write: {e}"),
            )
        })?;
        format!("file://{}", path.display())
    } else {
        state
            .documents
            .generate_policy_document(&policy_domain, &customer_domain, &product_domain)
            .await
            .map_err(|e| {
                error!(
                    quote_id = %quote_id,
                    customer_id = %quote.customer_id,
                    product_id = %quote.product_id,
                    error = %e,
                    error_debug = ?e,
                    "policy document generation/upload failed"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("pdf generation: {:#}", e),
                )
            })?
    };

    let row = sqlx::query_as::<_, PolicyRow>(
        r#"
        INSERT INTO policies
            (id, quote_id, policy_number, customer_id, product_id, state,
             premium, currency, start_date, end_date, pdf_url, coverage)
        VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8, $9, $10, $11)
        RETURNING id, quote_id, policy_number, customer_id, product_id, state,
                  premium, currency, start_date, end_date, pdf_url, coverage, created_at
        "#,
    )
    .bind(policy_id)
    .bind(quote.id)
    .bind(policy_number)
    .bind(quote.customer_id)
    .bind(quote.product_id)
    .bind(quote.premium)
    .bind(quote.currency)
    .bind(start_date)
    .bind(end_date)
    .bind(pdf_url)
    .bind(coverage)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("UPDATE quotes SET state = 'issued', updated_at = NOW() WHERE id = $1")
        .bind(quote_id)
        .execute(&**state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let event = InsuranceEvent::PolicyIssued(PolicyIssued {
        event_id: Uuid::new_v4(),
        occurred_at: Utc::now(),
        policy_id: row.id,
        quote_id: row.quote_id,
        customer_id: row.customer_id,
        product_id: row.product_id,
        premium: row.premium,
        currency: row.currency.clone(),
    });

    append_policy_version(&state, &row, "issued").await?;

    if let Err(err) = state.event_bus.publish(event.clone()).await {
        warn!(policy_id = %row.id, error = %err, "failed to publish PolicyIssued event");
    }

    Ok(Json(row))
}

/// GET /api/v1/policies/:id/document
/// Serves standalone-local PDFs directly. Production Blob-backed documents
/// are streamed through the API too, not redirected — the storage account
/// has public access disabled and no CORS rules configured (deliberately,
/// since these are policyholders' documents), so a browser can never
/// fetch a blob URL directly; the API is the only authenticated party
/// that can reach it.
pub async fn get_policy_document(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Response, (StatusCode, String)> {
    let row = sqlx::query("SELECT pdf_url FROM policies WHERE id = $1")
        .bind(id)
        .fetch_optional(&**state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".to_string()))?;

    let url: Option<String> = row.try_get("pdf_url").ok();
    let url = url.ok_or_else(|| (StatusCode::NOT_FOUND, "PDF not yet generated".to_string()))?;

    let bytes = if let Some(path) = url.strip_prefix("file://") {
        tokio::fs::read(path).await.map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                format!("policy PDF unavailable: {e}"),
            )
        })?
    } else {
        state.documents.download_by_url(&url).await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("policy PDF could not be retrieved from storage: {e}"),
            )
        })?
    };

    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=policy.pdf",
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}

pub async fn endorse_policy(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<EndorsePolicyRequest>,
) -> Result<Json<PolicyRow>, (StatusCode, String)> {
    require_roles(&user, LIFECYCLE_MUTATION_ROLES)?;
    if req.coverages.is_empty() || req.coverages.iter().any(|coverage| coverage.limit == 0) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "at least one valid coverage is required".to_string(),
        ));
    }
    let coverage = serde_json::to_value(&req.coverages)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let row = update_policy_state(
        &state,
        req.policy_id,
        &["active", "endorsed"],
        "endorsed",
        Some(coverage),
    )
    .await?;
    append_policy_version(&state, &row, "endorsement").await?;
    state.ledger.append(
        PolicyId(row.id),
        policy_ledger::LedgerEntryKind::Endorsed {
            endorsement_id: pas_domain::EndorsementId::new(),
        },
        Utc::now(),
        "rust-pas-sql",
    );
    Ok(Json(row))
}

pub async fn cancel_policy(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CancelPolicyRequest>,
) -> Result<Json<PolicyRow>, (StatusCode, String)> {
    require_roles(&user, LIFECYCLE_MUTATION_ROLES)?;
    if req.reason.trim().is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "cancellation reason is required".to_string(),
        ));
    }
    let row = update_policy_state(
        &state,
        req.policy_id,
        &["active", "endorsed"],
        "cancelled",
        None,
    )
    .await?;
    append_policy_version(&state, &row, "cancellation").await?;
    state.ledger.append(
        PolicyId(row.id),
        policy_ledger::LedgerEntryKind::Cancelled { reason: req.reason },
        Utc::now(),
        "rust-pas-sql",
    );
    Ok(Json(row))
}

pub async fn reinstate_policy(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<ReinstatePolicyRequest>,
) -> Result<Json<PolicyRow>, (StatusCode, String)> {
    require_roles(&user, LIFECYCLE_MUTATION_ROLES)?;
    let row = update_policy_state(
        &state,
        req.policy_id,
        &["cancelled", "canceled"],
        "active",
        None,
    )
    .await?;
    append_policy_version(&state, &row, "reinstatement").await?;
    state.ledger.append(
        PolicyId(row.id),
        policy_ledger::LedgerEntryKind::Reinstated,
        Utc::now(),
        "rust-pas-sql",
    );
    Ok(Json(row))
}

async fn update_policy_state(
    state: &AppState,
    policy_id: Uuid,
    allowed_states: &[&str],
    next_state: &str,
    coverage: Option<serde_json::Value>,
) -> Result<PolicyRow, (StatusCode, String)> {
    let current: String = sqlx::query_scalar("SELECT state FROM policies WHERE id = $1")
        .bind(policy_id)
        .fetch_optional(&**state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "policy not found".to_string()))?;
    if !allowed_states
        .iter()
        .any(|allowed| current.eq_ignore_ascii_case(allowed))
    {
        return Err((
            StatusCode::CONFLICT,
            format!("policy cannot transition from {current} to {next_state}"),
        ));
    }

    sqlx::query_as::<_, PolicyRow>(
        r#"
        UPDATE policies
        SET state = $2, coverage = COALESCE($3, coverage)
        WHERE id = $1
        RETURNING id, quote_id, policy_number, customer_id, product_id, state,
                  premium, currency, start_date, end_date, pdf_url, coverage, created_at
        "#,
    )
    .bind(policy_id)
    .bind(next_state)
    .bind(coverage)
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn append_policy_version(
    state: &AppState,
    row: &PolicyRow,
    source: &str,
) -> Result<(), (StatusCode, String)> {
    let line_of_business: String =
        sqlx::query_scalar("SELECT insurance_type FROM products WHERE id = $1")
            .bind(row.product_id)
            .fetch_one(&**state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .bitemporal_policy
        .append_version(
            PolicyId(row.id),
            PolicyVersionInput {
                policy_id: PolicyId(row.id),
                quote_id: QuoteId(row.quote_id),
                customer_id: CustomerId(row.customer_id),
                policy_number: row.policy_number.clone(),
                line_of_business,
                state: row.state.clone(),
                premium_cents: (row.premium * 100.0).round() as i64,
                currency: row.currency.clone(),
                coverage: row.coverage.clone(),
                effective_start: row.start_date.date_naive(),
                effective_end: Some(row.end_date.date_naive()),
                source: source.to_string(),
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(())
}

fn currency_from_code(code: &str) -> Currency {
    if code.eq_ignore_ascii_case("INR") {
        Currency::Inr
    } else if code.eq_ignore_ascii_case("AED") {
        Currency::Aed
    } else {
        Currency::Usd
    }
}

fn insurance_type_from_code(code: &str) -> domain::insurance::InsuranceType {
    match code {
        "life" => domain::insurance::InsuranceType::Life,
        "health" => domain::insurance::InsuranceType::Health,
        "property" => domain::insurance::InsuranceType::Property,
        "marine" => domain::insurance::InsuranceType::Marine,
        _ => domain::insurance::InsuranceType::Auto,
    }
}
