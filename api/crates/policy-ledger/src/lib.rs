use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, Utc};
use domain::ids::{CustomerId, PolicyId, QuoteId, UserId};
use infra::db::DbPool;
use pas_domain::EndorsementId;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEntryKind {
    QuoteCreated,
    Bound,
    Issued,
    Endorsed { endorsement_id: EndorsementId },
    Cancelled { reason: String },
    Reinstated,
    Expired,
    PremiumAdjusted { delta_cents: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Entry identifier. Reuses UserId as the workspace UUID newtype for now.
    pub id: UserId,
    pub policy_id: PolicyId,
    pub kind: LedgerEntryKind,
    /// When the event is effective in policy time.
    pub effective_at: DateTime<Utc>,
    /// When this entry was written to the ledger.
    pub recorded_at: DateTime<Utc>,
    /// Monotonic sequence number within policy_id.
    pub sequence: u64,
    /// Source system, e.g. rust-pas, migration, correction.
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyLedger {
    inner: Arc<Mutex<HashMap<PolicyId, Vec<LedgerEntry>>>>,
}

impl PolicyLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(
        &self,
        policy_id: PolicyId,
        kind: LedgerEntryKind,
        effective_at: DateTime<Utc>,
        source: &str,
    ) -> LedgerEntry {
        let mut entries_by_policy = self.inner.lock().expect("policy ledger mutex poisoned");
        let entries = entries_by_policy.entry(policy_id).or_default();
        let sequence = entries
            .last()
            .map(|entry| entry.sequence.checked_add(1).unwrap_or(u64::MAX))
            .unwrap_or(1);

        let entry = LedgerEntry {
            id: UserId::new(),
            policy_id,
            kind,
            effective_at,
            recorded_at: Utc::now(),
            sequence,
            source: source.to_string(),
        };
        entries.push(entry.clone());
        entry
    }

    pub fn entries(&self, policy_id: &PolicyId) -> Vec<LedgerEntry> {
        let entries_by_policy = self.inner.lock().expect("policy ledger mutex poisoned");
        let mut entries = entries_by_policy
            .get(policy_id)
            .cloned()
            .unwrap_or_default();
        sort_entries(&mut entries);
        entries
    }

    pub fn as_of(&self, policy_id: &PolicyId, as_of: DateTime<Utc>) -> Vec<LedgerEntry> {
        let entries_by_policy = self.inner.lock().expect("policy ledger mutex poisoned");
        let mut entries: Vec<LedgerEntry> = entries_by_policy
            .get(policy_id)
            .into_iter()
            .flatten()
            .filter(|entry| entry.effective_at <= as_of)
            .cloned()
            .collect();
        sort_entries(&mut entries);
        entries
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LedgerError {
    #[error("policy was not found")]
    PolicyNotFound,
    #[error("ledger entry sequence overflow")]
    SequenceOverflow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyVersionInput {
    pub policy_id: PolicyId,
    pub quote_id: QuoteId,
    pub customer_id: CustomerId,
    pub policy_number: String,
    pub line_of_business: String,
    pub state: String,
    pub premium_cents: i64,
    pub currency: String,
    pub coverage: serde_json::Value,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyVersion {
    pub version_id: Uuid,
    pub policy_id: PolicyId,
    pub quote_id: QuoteId,
    pub customer_id: CustomerId,
    pub policy_number: String,
    pub line_of_business: String,
    pub state: String,
    pub premium_cents: i64,
    pub currency: String,
    pub coverage: serde_json::Value,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
    pub sys_start: DateTime<Utc>,
    pub sys_end: Option<DateTime<Utc>>,
    pub version_seq: i64,
    pub source: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PolicyLedgerError {
    #[error("database error: {0}")]
    DatabaseError(String),
    #[error("policy was not found")]
    PolicyNotFound,
    #[error("version conflict: {0}")]
    VersionConflict(String),
}

impl From<sqlx::Error> for PolicyLedgerError {
    fn from(value: sqlx::Error) -> Self {
        Self::DatabaseError(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct BiTemporalPolicyStore {
    pool: Arc<DbPool>,
}

impl BiTemporalPolicyStore {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    pub async fn append_version(
        &self,
        policy_id: PolicyId,
        input: PolicyVersionInput,
    ) -> Result<PolicyVersion, PolicyLedgerError> {
        if input.policy_id != policy_id {
            return Err(PolicyLedgerError::VersionConflict(format!(
                "input policy_id {} does not match append policy_id {}",
                input.policy_id, policy_id
            )));
        }

        let mut tx = self.pool.begin().await?;

        let version_seq: i64 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(MAX(version_seq), 0) + 1
            FROM policy_versions
            WHERE policy_id = $1
            "#,
        )
        .bind(policy_id.0)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE policy_versions
            SET sys_end = NOW()
            WHERE policy_id = $1 AND sys_end IS NULL
            "#,
        )
        .bind(policy_id.0)
        .execute(&mut *tx)
        .await?;

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
        .bind(policy_id.0)
        .bind(input.quote_id.0)
        .bind(input.customer_id.0)
        .bind(input.policy_number)
        .bind(input.line_of_business)
        .bind(input.state)
        .bind(input.premium_cents)
        .bind(input.currency)
        .bind(input.coverage)
        .bind(input.effective_start)
        .bind(input.effective_end)
        .bind(version_seq)
        .bind(input.source)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        policy_version_from_row(&row)
    }

    pub async fn current_version(
        &self,
        policy_id: PolicyId,
    ) -> Result<Option<PolicyVersion>, PolicyLedgerError> {
        let row = sqlx::query(
            r#"
            SELECT version_id, policy_id, quote_id, customer_id, policy_number,
                   line_of_business, state, premium_cents, currency, coverage,
                   effective_start, effective_end, sys_start, sys_end,
                   version_seq, source
            FROM policy_versions
            WHERE policy_id = $1 AND sys_end IS NULL
            ORDER BY version_seq DESC
            LIMIT 1
            "#,
        )
        .bind(policy_id.0)
        .fetch_optional(&**self.pool)
        .await?;

        row.map(|row| policy_version_from_row(&row)).transpose()
    }

    pub async fn as_of_business_date(
        &self,
        policy_id: PolicyId,
        as_of: NaiveDate,
    ) -> Result<Option<PolicyVersion>, PolicyLedgerError> {
        let row = sqlx::query(
            r#"
            SELECT version_id, policy_id, quote_id, customer_id, policy_number,
                   line_of_business, state, premium_cents, currency, coverage,
                   effective_start, effective_end, sys_start, sys_end,
                   version_seq, source
            FROM policy_versions
            WHERE policy_id = $1
              AND effective_start <= $2
              AND (effective_end IS NULL OR effective_end > $2)
              AND sys_end IS NULL
            ORDER BY version_seq DESC
            LIMIT 1
            "#,
        )
        .bind(policy_id.0)
        .bind(as_of)
        .fetch_optional(&**self.pool)
        .await?;

        row.map(|row| policy_version_from_row(&row)).transpose()
    }

    pub async fn as_of_system_time(
        &self,
        policy_id: PolicyId,
        sys_as_of: DateTime<Utc>,
    ) -> Result<Option<PolicyVersion>, PolicyLedgerError> {
        let row = sqlx::query(
            r#"
            SELECT version_id, policy_id, quote_id, customer_id, policy_number,
                   line_of_business, state, premium_cents, currency, coverage,
                   effective_start, effective_end, sys_start, sys_end,
                   version_seq, source
            FROM policy_versions
            WHERE policy_id = $1
              AND sys_start <= $2
              AND (sys_end IS NULL OR sys_end > $2)
            ORDER BY version_seq DESC
            LIMIT 1
            "#,
        )
        .bind(policy_id.0)
        .bind(sys_as_of)
        .fetch_optional(&**self.pool)
        .await?;

        row.map(|row| policy_version_from_row(&row)).transpose()
    }

    pub async fn full_history(
        &self,
        policy_id: PolicyId,
    ) -> Result<Vec<PolicyVersion>, PolicyLedgerError> {
        let rows = sqlx::query(
            r#"
            SELECT version_id, policy_id, quote_id, customer_id, policy_number,
                   line_of_business, state, premium_cents, currency, coverage,
                   effective_start, effective_end, sys_start, sys_end,
                   version_seq, source
            FROM policy_versions
            WHERE policy_id = $1
            ORDER BY version_seq ASC, sys_start ASC
            "#,
        )
        .bind(policy_id.0)
        .fetch_all(&**self.pool)
        .await?;

        rows.iter().map(policy_version_from_row).collect()
    }
}

fn policy_version_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<PolicyVersion, PolicyLedgerError> {
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

fn sort_entries(entries: &mut [LedgerEntry]) {
    entries.sort_by_key(|entry| (entry.effective_at, entry.sequence));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn test_input(policy_id: PolicyId, effective_start: NaiveDate) -> PolicyVersionInput {
        PolicyVersionInput {
            policy_id,
            quote_id: QuoteId::new(),
            customer_id: CustomerId::new(),
            policy_number: format!("POL-{}", policy_id.0.simple()),
            line_of_business: "auto".to_string(),
            state: "active".to_string(),
            premium_cents: 125_000,
            currency: "USD".to_string(),
            coverage: serde_json::json!({"bodily_injury": 100000}),
            effective_start,
            effective_end: None,
            source: "rust-pas-test".to_string(),
        }
    }

    async fn test_store() -> Option<BiTemporalPolicyStore> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        let pool = Arc::new(DbPool::connect_lazy(&url).ok()?);
        sqlx::raw_sql(include_str!(
            "../../../migrations/005_bitemporal_policy_ledger.sql"
        ))
        .execute(&**pool)
        .await
        .ok()?;
        Some(BiTemporalPolicyStore::new(pool))
    }

    #[test]
    fn test_append_and_retrieve() {
        let ledger = PolicyLedger::new();
        let policy_id = PolicyId::new();

        ledger.append(
            policy_id,
            LedgerEntryKind::Issued,
            ts(2026, 1, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            LedgerEntryKind::Cancelled {
                reason: "insured request".to_string(),
            },
            ts(2026, 3, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            LedgerEntryKind::Endorsed {
                endorsement_id: EndorsementId::new(),
            },
            ts(2026, 2, 1),
            "rust-pas",
        );

        let entries = ledger.entries(&policy_id);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].effective_at, ts(2026, 1, 1));
        assert_eq!(entries[1].effective_at, ts(2026, 2, 1));
        assert_eq!(entries[2].effective_at, ts(2026, 3, 1));
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[1].sequence, 3);
        assert_eq!(entries[2].sequence, 2);
    }

    #[test]
    fn test_out_of_sequence() {
        let ledger = PolicyLedger::new();
        let policy_id = PolicyId::new();

        ledger.append(
            policy_id,
            LedgerEntryKind::Issued,
            ts(2026, 1, 1),
            "rust-pas",
        );
        let endorsement_id = EndorsementId::new();
        ledger.append(
            policy_id,
            LedgerEntryKind::Endorsed { endorsement_id },
            ts(2025, 12, 15),
            "correction",
        );

        let dec_31 = ledger.as_of(&policy_id, ts(2025, 12, 31));
        assert_eq!(dec_31.len(), 1);
        assert!(matches!(dec_31[0].kind, LedgerEntryKind::Endorsed { .. }));

        let jan_2 = ledger.as_of(&policy_id, ts(2026, 1, 2));
        assert_eq!(jan_2.len(), 2);
        assert!(matches!(jan_2[0].kind, LedgerEntryKind::Endorsed { .. }));
        assert_eq!(jan_2[0].effective_at, ts(2025, 12, 15));
        assert_eq!(jan_2[1].kind, LedgerEntryKind::Issued);
        assert_eq!(jan_2[1].effective_at, ts(2026, 1, 1));
    }

    #[test]
    fn test_as_of_empty() {
        let ledger = PolicyLedger::new();
        let entries = ledger.as_of(&PolicyId::new(), ts(2026, 1, 1));
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_append_creates_version() {
        let Some(store) = test_store().await else {
            return;
        };
        let policy_id = PolicyId::new();
        let input = test_input(policy_id, date(2026, 1, 1));

        let appended = store.append_version(policy_id, input).await.unwrap();
        let current = store.current_version(policy_id).await.unwrap().unwrap();

        assert_eq!(current.version_id, appended.version_id);
        assert_eq!(current.policy_id, policy_id);
        assert_eq!(current.version_seq, 1);
        assert!(current.sys_end.is_none());
    }

    #[tokio::test]
    async fn test_append_closes_previous() {
        let Some(store) = test_store().await else {
            return;
        };
        let policy_id = PolicyId::new();
        let first = store
            .append_version(policy_id, test_input(policy_id, date(2026, 1, 1)))
            .await
            .unwrap();
        let mut second_input = test_input(policy_id, date(2026, 2, 1));
        second_input.state = "endorsed".to_string();
        let second = store.append_version(policy_id, second_input).await.unwrap();

        let first_as_written = store
            .as_of_system_time(policy_id, first.sys_start + Duration::milliseconds(1))
            .await
            .unwrap()
            .unwrap();
        let current = store.current_version(policy_id).await.unwrap().unwrap();
        let history = store.full_history(policy_id).await.unwrap();

        assert_eq!(first_as_written.version_id, first.version_id);
        assert!(history[0].sys_end.is_some());
        assert_eq!(current.version_id, second.version_id);
        assert!(current.sys_end.is_none());
    }

    #[tokio::test]
    async fn test_as_of_business_date() {
        let Some(store) = test_store().await else {
            return;
        };
        let policy_id = PolicyId::new();
        let appended = store
            .append_version(policy_id, test_input(policy_id, date(2026, 1, 1)))
            .await
            .unwrap();

        let june = store
            .as_of_business_date(policy_id, date(2026, 6, 1))
            .await
            .unwrap();
        let before = store
            .as_of_business_date(policy_id, date(2025, 12, 31))
            .await
            .unwrap();

        assert_eq!(june.unwrap().version_id, appended.version_id);
        assert!(before.is_none());
    }

    #[tokio::test]
    async fn test_as_of_system_time() {
        let Some(store) = test_store().await else {
            return;
        };
        let policy_id = PolicyId::new();
        let before_append = Utc::now() - Duration::seconds(1);
        let appended = store
            .append_version(policy_id, test_input(policy_id, date(2026, 1, 1)))
            .await
            .unwrap();
        let after_append = appended.sys_start + Duration::milliseconds(1);

        let before = store
            .as_of_system_time(policy_id, before_append)
            .await
            .unwrap();
        let after = store
            .as_of_system_time(policy_id, after_append)
            .await
            .unwrap();

        assert!(before.is_none());
        assert_eq!(after.unwrap().version_id, appended.version_id);
    }

    #[tokio::test]
    async fn test_full_history() {
        let Some(store) = test_store().await else {
            return;
        };
        let policy_id = PolicyId::new();
        for month in 1..=3 {
            let mut input = test_input(policy_id, date(2026, month, 1));
            input.state = format!("version-{month}");
            store.append_version(policy_id, input).await.unwrap();
        }

        let history = store.full_history(policy_id).await.unwrap();

        assert_eq!(history.len(), 3);
        assert_eq!(history[0].version_seq, 1);
        assert_eq!(history[1].version_seq, 2);
        assert_eq!(history[2].version_seq, 3);
        assert_eq!(history[0].state, "version-1");
        assert_eq!(history[2].state, "version-3");
    }
}
