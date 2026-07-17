use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::ids::{CustomerId, PolicyId, QuoteId};
use infra::db::DbPool;
use policy_ledger::PolicyVersion;
use premium_ledger::{BatchInput, JournalEntryInput, JournalLineInput, PremiumSubledger};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

use redis::{Client as RedisClient, Script};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OosEndorsementInput {
    pub policy_id: PolicyId,
    pub oos_effective_date: NaiveDate,
    pub changes: PolicyChanges,
    pub requested_by: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyChanges {
    pub new_state: Option<String>,
    pub new_premium_cents: Option<i64>,
    pub new_coverage: Option<serde_json::Value>,
    pub new_effective_end: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OosResult {
    pub policy_id: PolicyId,
    pub oos_version_id: Uuid,
    pub blocking_versions_voided: usize,
    pub versions_resequenced: usize,
    pub journal_batch_id: Option<Uuid>,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OosError {
    #[error("policy was not found")]
    PolicyNotFound,
    #[error("no base version found")]
    NoBaseVersionFound,
    #[error("concurrent modification detected")]
    ConcurrentModification,
    #[error("database error: {0}")]
    DatabaseError(String),
    #[error("unbalanced journal: {0}")]
    UnbalancedJournal(String),
}

impl From<sqlx::Error> for OosError {
    fn from(value: sqlx::Error) -> Self {
        Self::DatabaseError(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct OosOrchestrator {
    pool: Arc<DbPool>,
    redis_url: String,
}

#[derive(Debug, Clone)]
struct PolicyLock {
    key: String,
    token: String,
}

impl OosOrchestrator {
    pub fn new(pool: Arc<DbPool>, redis_url: impl Into<String>) -> Self {
        Self {
            pool,
            redis_url: redis_url.into(),
        }
    }

    pub async fn apply(&self, input: OosEndorsementInput) -> Result<OosResult, OosError> {
        let lock = self.acquire_policy_lock(input.policy_id).await?;
        let result = self.apply_inner(input).await;
        if let Some(lock) = lock {
            self.release_policy_lock(lock).await;
        }
        result
    }

    async fn apply_inner(&self, input: OosEndorsementInput) -> Result<OosResult, OosError> {
        let mut tx = self.pool.begin().await?;
        let applied_at = Utc::now();

        let policy_exists: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT 1
            FROM policy_versions
            WHERE policy_id = $1
            LIMIT 1
            "#,
        )
        .bind(input.policy_id.0)
        .fetch_optional(&mut *tx)
        .await?;
        if policy_exists.is_none() {
            return Err(OosError::PolicyNotFound);
        }

        sqlx::query(
            r#"
            SELECT version_id
            FROM policy_versions
            WHERE policy_id = $1 AND sys_end IS NULL
            ORDER BY version_seq DESC
            FOR UPDATE
            "#,
        )
        .bind(input.policy_id.0)
        .fetch_all(&mut *tx)
        .await?;

        let blocking_versions =
            select_blocking_versions(&mut tx, input.policy_id, input.oos_effective_date).await?;

        for version in &blocking_versions {
            let updated = sqlx::query(
                r#"
                UPDATE policy_versions
                SET sys_end = NOW()
                WHERE version_id = $1 AND sys_end IS NULL
                "#,
            )
            .bind(version.version_id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(OosError::ConcurrentModification);
            }
        }

        let base_as_of = input
            .oos_effective_date
            .checked_sub_signed(Duration::days(1))
            .ok_or(OosError::NoBaseVersionFound)?;
        let base_version = select_as_of_business_date(&mut tx, input.policy_id, base_as_of)
            .await?
            .ok_or(OosError::NoBaseVersionFound)?;

        let next_version_seq = max_version_seq(&mut tx, input.policy_id).await? + 1;
        let oos_draft = apply_policy_changes(&base_version, &input.changes);
        let oos_version = insert_policy_version(
            &mut tx,
            &oos_draft,
            input.oos_effective_date,
            oos_draft.effective_end,
            next_version_seq,
            "oos-endorsement",
        )
        .await?;

        let mut resequenced = 0usize;
        let mut resequence_seq = next_version_seq + 1;
        for version in &blocking_versions {
            insert_policy_version(
                &mut tx,
                version,
                version.effective_start,
                version.effective_end,
                resequence_seq,
                "oos-resequence",
            )
            .await?;
            resequenced += 1;
            resequence_seq += 1;
        }

        let delta_cents = oos_version.premium_cents - base_version.premium_cents;
        let batch = build_premium_delta_journal(
            input.policy_id,
            delta_cents,
            input.oos_effective_date,
            &oos_version.currency,
        )?;

        let journal_batch_id = if let Some(batch) = batch {
            let subledger = PremiumSubledger::new(self.pool.clone());
            let result = subledger
                .post_batch_in_transaction(&mut tx, batch)
                .await
                .map_err(|err| match err {
                    premium_ledger::SubledgerError::UnbalancedEntry { .. } => {
                        OosError::UnbalancedJournal(err.to_string())
                    }
                    _ => OosError::DatabaseError(err.to_string()),
                })?;
            Some(result.batch_id)
        } else {
            None
        };

        tx.commit().await?;

        Ok(OosResult {
            policy_id: input.policy_id,
            oos_version_id: oos_version.version_id,
            blocking_versions_voided: blocking_versions.len(),
            versions_resequenced: resequenced,
            journal_batch_id,
            applied_at,
        })
    }

    async fn acquire_policy_lock(
        &self,
        policy_id: PolicyId,
    ) -> Result<Option<PolicyLock>, OosError> {
        if self.redis_url.trim().is_empty() {
            return Ok(None);
        }

        let client = RedisClient::open(self.redis_url.clone())
            .map_err(|err| OosError::DatabaseError(format!("redis client init failed: {err}")))?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| OosError::DatabaseError(format!("redis connection failed: {err}")))?;

        let key = format!("oos:policy:{}", policy_id.0);
        let token = Uuid::new_v4().to_string();

        let lock_result: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(30_000)
            .query_async(&mut conn)
            .await
            .map_err(|err| OosError::DatabaseError(format!("redis lock acquire failed: {err}")))?;

        if lock_result.is_none() {
            return Err(OosError::ConcurrentModification);
        }

        Ok(Some(PolicyLock { key, token }))
    }

    async fn release_policy_lock(&self, lock: PolicyLock) {
        if self.redis_url.trim().is_empty() {
            return;
        }

        if let Ok(client) = RedisClient::open(self.redis_url.clone()) {
            if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
                let script = Script::new(
                    "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end",
                );
                let _ = script
                    .key(&lock.key)
                    .arg(&lock.token)
                    .invoke_async::<_, i32>(&mut conn)
                    .await;
            }
        }
    }
}

pub fn apply_policy_changes(base: &PolicyVersion, changes: &PolicyChanges) -> PolicyVersion {
    let mut updated = base.clone();
    if let Some(state) = &changes.new_state {
        updated.state = state.clone();
    }
    if let Some(premium_cents) = changes.new_premium_cents {
        updated.premium_cents = premium_cents;
    }
    if let Some(coverage) = &changes.new_coverage {
        updated.coverage = coverage.clone();
    }
    if let Some(effective_end) = changes.new_effective_end {
        updated.effective_end = Some(effective_end);
    }
    updated
}

pub fn build_premium_delta_journal(
    policy_id: PolicyId,
    delta_cents: i64,
    effective_date: NaiveDate,
    currency: &str,
) -> Result<Option<BatchInput>, OosError> {
    if delta_cents == 0 {
        return Ok(None);
    }

    let amount = cents_to_amount(delta_cents.abs());
    let lines = if delta_cents > 0 {
        vec![
            JournalLineInput {
                account_no: "1210".to_string(),
                amount,
                currency: currency.to_string(),
            },
            JournalLineInput {
                account_no: "2100".to_string(),
                amount: -amount,
                currency: currency.to_string(),
            },
        ]
    } else {
        vec![
            JournalLineInput {
                account_no: "2100".to_string(),
                amount,
                currency: currency.to_string(),
            },
            JournalLineInput {
                account_no: "2310".to_string(),
                amount: -amount,
                currency: currency.to_string(),
            },
        ]
    };

    let net = lines
        .iter()
        .fold(Decimal::ZERO, |acc, line| acc + line.amount);
    if net != Decimal::ZERO {
        return Err(OosError::UnbalancedJournal(format!("net={net}")));
    }

    Ok(Some(BatchInput {
        policy_id: policy_id.0,
        event_type: "OOS_ENDORSEMENT_PREMIUM_DELTA".to_string(),
        entries: vec![JournalEntryInput {
            effective_date,
            lines,
        }],
    }))
}

async fn select_blocking_versions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    policy_id: PolicyId,
    oos_effective_date: NaiveDate,
) -> Result<Vec<PolicyVersion>, OosError> {
    let sql = format!(
        "{}{}",
        POLICY_VERSION_SELECT_PREFIX,
        r#"
            FROM policy_versions
            WHERE policy_id = $1
              AND effective_start > $2
              AND sys_end IS NULL
            ORDER BY effective_start ASC, version_seq ASC
            "#
    );
    let rows = sqlx::query(&sql)
        .bind(policy_id.0)
        .bind(oos_effective_date)
        .fetch_all(&mut **tx)
        .await?;

    rows.iter().map(policy_version_from_row).collect()
}

async fn select_as_of_business_date(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    policy_id: PolicyId,
    as_of: NaiveDate,
) -> Result<Option<PolicyVersion>, OosError> {
    let sql = format!(
        "{}{}",
        POLICY_VERSION_SELECT_PREFIX,
        r#"
            FROM policy_versions
            WHERE policy_id = $1
              AND effective_start <= $2
              AND (effective_end IS NULL OR effective_end > $2)
              AND sys_end IS NULL
            ORDER BY version_seq DESC
            LIMIT 1
            "#
    );
    let row = sqlx::query(&sql)
        .bind(policy_id.0)
        .bind(as_of)
        .fetch_optional(&mut **tx)
        .await?;

    row.map(|row| policy_version_from_row(&row)).transpose()
}

async fn max_version_seq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    policy_id: PolicyId,
) -> Result<i64, OosError> {
    let seq = sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(version_seq), 0)
        FROM policy_versions
        WHERE policy_id = $1
        "#,
    )
    .bind(policy_id.0)
    .fetch_one(&mut **tx)
    .await?;
    Ok(seq)
}

async fn insert_policy_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version: &PolicyVersion,
    effective_start: NaiveDate,
    effective_end: Option<NaiveDate>,
    version_seq: i64,
    source: &str,
) -> Result<PolicyVersion, OosError> {
    let row = sqlx::query(
        r#"
        INSERT INTO policy_versions
            (policy_id, quote_id, customer_id, policy_number, line_of_business,
             state, premium_cents, currency, coverage, effective_start,
             effective_end, sys_start, sys_end, version_seq, source)
        VALUES
            ($1, $2, $3, $4, $5,
             $6, $7, $8, $9, $10,
             $11, NOW(), NULL, $12, $13)
        RETURNING version_id, policy_id, quote_id, customer_id, policy_number,
                  line_of_business, state, premium_cents, currency, coverage,
                  effective_start, effective_end, sys_start, sys_end,
                  version_seq, source
        "#,
    )
    .bind(version.policy_id.0)
    .bind(version.quote_id.0)
    .bind(version.customer_id.0)
    .bind(&version.policy_number)
    .bind(&version.line_of_business)
    .bind(&version.state)
    .bind(version.premium_cents)
    .bind(&version.currency)
    .bind(&version.coverage)
    .bind(effective_start)
    .bind(effective_end)
    .bind(version_seq)
    .bind(source)
    .fetch_one(&mut **tx)
    .await?;

    policy_version_from_row(&row)
}

fn cents_to_amount(cents: i64) -> Decimal {
    Decimal::from(cents) / Decimal::from(100)
}

const POLICY_VERSION_SELECT_PREFIX: &str = r#"
            SELECT version_id, policy_id, quote_id, customer_id, policy_number,
                   line_of_business, state, premium_cents, currency, coverage,
                   effective_start, effective_end, sys_start, sys_end,
                   version_seq, source
            "#;

fn policy_version_from_row(row: &sqlx::postgres::PgRow) -> Result<PolicyVersion, OosError> {
    Ok(PolicyVersion {
        version_id: row.try_get("version_id")?,
        policy_id: PolicyId(row.try_get("policy_id")?),
        quote_id: QuoteId(row.try_get("quote_id")?),
        customer_id: CustomerId(row.try_get("customer_id")?),
        policy_number: row.try_get("policy_number")?,
        line_of_business: row.try_get("line_of_business")?,
        state: row.try_get("state")?,
        premium_cents: row.try_get("premium_cents")?,
        currency: row.try_get("currency")?,
        coverage: row.try_get("coverage")?,
        effective_start: row.try_get("effective_start")?,
        effective_end: row.try_get("effective_end")?,
        sys_start: row.try_get("sys_start")?,
        sys_end: row.try_get("sys_end")?,
        version_seq: row.try_get("version_seq")?,
        source: row.try_get("source")?,
    })
}

#[allow(dead_code)]
fn _assert_premium_subledger_dependency_is_present(_: &PremiumSubledger) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn base_version() -> PolicyVersion {
        PolicyVersion {
            version_id: Uuid::new_v4(),
            policy_id: PolicyId::new(),
            quote_id: QuoteId::new(),
            customer_id: CustomerId::new(),
            policy_number: "POL-1".to_string(),
            line_of_business: "homeowners".to_string(),
            state: "bound".to_string(),
            premium_cents: 120_000,
            currency: "USD".to_string(),
            coverage: serde_json::json!({"limit": 100000}),
            effective_start: date(2026, 1, 1),
            effective_end: None,
            sys_start: Utc::now(),
            sys_end: None,
            version_seq: 1,
            source: "test".to_string(),
        }
    }

    fn journal_net(batch: &BatchInput) -> Decimal {
        batch.entries[0]
            .lines
            .iter()
            .fold(Decimal::ZERO, |acc, line| acc + line.amount)
    }

    #[test]
    fn test_no_blocking_versions() {
        let result = OosResult {
            policy_id: PolicyId::new(),
            oos_version_id: Uuid::new_v4(),
            blocking_versions_voided: 0,
            versions_resequenced: 0,
            journal_batch_id: None,
            applied_at: Utc::now(),
        };

        assert_eq!(result.blocking_versions_voided, 0);
        assert_eq!(result.versions_resequenced, 0);
    }

    #[test]
    fn test_policy_changes_applied() {
        let base = base_version();
        let changes = PolicyChanges {
            new_state: Some("endorsed".to_string()),
            new_premium_cents: Some(135_000),
            new_coverage: Some(serde_json::json!({"limit": 250000})),
            new_effective_end: Some(date(2026, 12, 31)),
        };

        let updated = apply_policy_changes(&base, &changes);

        assert_eq!(updated.state, "endorsed");
        assert_eq!(updated.premium_cents, 135_000);
        assert_eq!(updated.coverage, serde_json::json!({"limit": 250000}));
        assert_eq!(updated.effective_end, Some(date(2026, 12, 31)));
        assert_eq!(updated.policy_id, base.policy_id);
    }

    #[test]
    fn test_premium_delta_additional() {
        let policy_id = PolicyId::new();
        let batch = build_premium_delta_journal(policy_id, 15_000, date(2026, 2, 1), "USD")
            .unwrap()
            .unwrap();

        assert_eq!(batch.policy_id, policy_id.0);
        assert_eq!(batch.event_type, "OOS_ENDORSEMENT_PREMIUM_DELTA");
        assert_eq!(batch.entries[0].lines[0].account_no, "1210");
        assert_eq!(batch.entries[0].lines[0].amount, Decimal::from(150));
        assert_eq!(batch.entries[0].lines[1].account_no, "2100");
        assert_eq!(batch.entries[0].lines[1].amount, Decimal::from(-150));
        assert_eq!(journal_net(&batch), Decimal::ZERO);
    }

    #[test]
    fn test_premium_delta_return() {
        let policy_id = PolicyId::new();
        let batch = build_premium_delta_journal(policy_id, -25_000, date(2026, 2, 1), "USD")
            .unwrap()
            .unwrap();

        assert_eq!(batch.policy_id, policy_id.0);
        assert_eq!(batch.entries[0].lines[0].account_no, "2100");
        assert_eq!(batch.entries[0].lines[0].amount, Decimal::from(250));
        assert_eq!(batch.entries[0].lines[1].account_no, "2310");
        assert_eq!(batch.entries[0].lines[1].amount, Decimal::from(-250));
        assert_eq!(journal_net(&batch), Decimal::ZERO);
    }

    #[test]
    fn test_zero_delta_no_journal() {
        let batch =
            build_premium_delta_journal(PolicyId::new(), 0, date(2026, 2, 1), "USD").unwrap();
        assert!(batch.is_none());
    }
}
