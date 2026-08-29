use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use sqlx::Row;

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct DashboardStats {
    pub total_quoted: i64,
    pub total_bound: i64,
    pub total_issued: i64,
    pub total_cancelled: i64,
    pub total_active_policies: i64,
    pub total_claims: i64,
    pub total_paid_claims: i64,
}

#[derive(Debug, Serialize)]
pub struct DashboardHealth {
    pub status: String,
    pub total: i64,
}

/// GET /api/v1/dashboard/stats
pub async fn stats(
    State(state): State<AppState>,
) -> Result<Json<DashboardStats>, (StatusCode, String)> {
    let quote_counts = sqlx::query(
        r#"
        SELECT
            COUNT(*)::BIGINT AS total_quoted,
            COUNT(*) FILTER (WHERE state = 'bound')::BIGINT AS total_bound,
            COUNT(*) FILTER (WHERE state = 'issued')::BIGINT AS total_issued,
            COUNT(*) FILTER (WHERE state = 'cancelled')::BIGINT AS total_cancelled
        FROM quotes
        "#,
    )
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let policy_count =
        sqlx::query(r#"SELECT COUNT(*)::BIGINT AS n FROM policies WHERE state = 'active'"#)
            .fetch_one(&**state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // sagepas's claims table is `policy_claims` (status column, not `claims`/state
    // like sesure-us's dashboard queried) -- see migration 007_policy_workspace.sql.
    // No code path transitions a claim's status away from its 'reported' default yet
    // (payment execution against claims is explicitly out of scope per migration
    // 015's own comment), so total_paid_claims is a real, correctly-scoped query
    // that returns 0 today rather than a fabricated number.
    let claim_counts = sqlx::query(
        r#"
        SELECT
            COUNT(*)::BIGINT AS total_claims,
            COUNT(*) FILTER (WHERE status IN ('closed', 'settled', 'paid'))::BIGINT AS total_paid
        FROM policy_claims
        "#,
    )
    .fetch_one(&**state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DashboardStats {
        total_quoted: quote_counts.try_get("total_quoted").unwrap_or(0),
        total_bound: quote_counts.try_get("total_bound").unwrap_or(0),
        total_issued: quote_counts.try_get("total_issued").unwrap_or(0),
        total_cancelled: quote_counts.try_get("total_cancelled").unwrap_or(0),
        total_active_policies: policy_count.try_get("n").unwrap_or(0),
        total_claims: claim_counts.try_get("total_claims").unwrap_or(0),
        total_paid_claims: claim_counts.try_get("total_paid").unwrap_or(0),
    }))
}

/// GET /api/v1/health/claims
pub async fn claims_health(
    State(state): State<AppState>,
) -> Result<Json<DashboardHealth>, (StatusCode, String)> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS n FROM policy_claims")
        .fetch_one(&**state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DashboardHealth {
        status: "ok".to_string(),
        total: row.try_get("n").unwrap_or(0),
    }))
}

/// GET /api/v1/health/policies
pub async fn policies_health(
    State(state): State<AppState>,
) -> Result<Json<DashboardHealth>, (StatusCode, String)> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS n FROM policies")
        .fetch_one(&**state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DashboardHealth {
        status: "ok".to_string(),
        total: row.try_get("n").unwrap_or(0),
    }))
}
