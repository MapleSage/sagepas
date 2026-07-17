use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, Utc};
use domain::ids::PolicyId;
use infra::db::DbPool;
use pas_domain::EndorsementId;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PremiumEntryKind {
    WrittenPremium,
    PremiumAdjustment {
        delta_cents: i64,
    },
    EndorsementAdjustment {
        endorsement_id: EndorsementId,
        delta_cents: i64,
    },
    CancellationRefund {
        amount_cents: i64,
    },
    ReinstatementCharge {
        amount_cents: i64,
    },
    EarnedPremiumSnapshot {
        amount_cents: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremiumEntry {
    pub id: Uuid,
    pub policy_id: PolicyId,
    pub kind: PremiumEntryKind,
    pub effective_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub sequence: u64,
    /// Source system, e.g. rust-pas, pricing, migration.
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct PremiumLedger {
    inner: Arc<Mutex<HashMap<PolicyId, Vec<PremiumEntry>>>>,
}

impl PremiumLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(
        &self,
        policy_id: PolicyId,
        kind: PremiumEntryKind,
        effective_at: DateTime<Utc>,
        source: &str,
    ) -> PremiumEntry {
        let mut entries_by_policy = self.inner.lock().expect("premium ledger mutex poisoned");
        let entries = entries_by_policy.entry(policy_id).or_default();
        let sequence = entries
            .last()
            .map(|entry| entry.sequence.checked_add(1).unwrap_or(u64::MAX))
            .unwrap_or(1);

        let entry = PremiumEntry {
            id: Uuid::new_v4(),
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

    pub fn entries(&self, policy_id: &PolicyId) -> Vec<PremiumEntry> {
        let entries_by_policy = self.inner.lock().expect("premium ledger mutex poisoned");
        let mut entries = entries_by_policy
            .get(policy_id)
            .cloned()
            .unwrap_or_default();
        sort_entries(&mut entries);
        entries
    }

    pub fn as_of(&self, policy_id: &PolicyId, as_of: DateTime<Utc>) -> Vec<PremiumEntry> {
        let entries_by_policy = self.inner.lock().expect("premium ledger mutex poisoned");
        let mut entries: Vec<PremiumEntry> = entries_by_policy
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

#[derive(Debug, Clone)]
pub struct PremiumSubledger {
    pool: Arc<DbPool>,
}

impl PremiumSubledger {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    pub async fn post_batch(&self, input: BatchInput) -> Result<BatchResult, SubledgerError> {
        let mut tx = self.pool.begin().await?;
        let result = self.post_batch_in_transaction(&mut tx, input).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn post_batch_in_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        input: BatchInput,
    ) -> Result<BatchResult, SubledgerError> {
        for (entry_index, entry) in input.entries.iter().enumerate() {
            validate_entry_balance(entry_index, entry)?;
        }

        let batch_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO accounting_batches (policy_id, event_type)
            VALUES ($1, $2)
            RETURNING batch_id
            "#,
        )
        .bind(input.policy_id)
        .bind(&input.event_type)
        .fetch_one(&mut **tx)
        .await?;

        let mut total_lines: usize = 0;
        for entry in &input.entries {
            let entry_id: Uuid = sqlx::query_scalar(
                r#"
                INSERT INTO journal_entries (batch_id, effective_date)
                VALUES ($1, $2)
                RETURNING entry_id
                "#,
            )
            .bind(batch_id)
            .bind(entry.effective_date)
            .fetch_one(&mut **tx)
            .await?;

            for line in &entry.lines {
                let is_leaf = account_leaf_status(tx, &line.account_no).await?;
                if !is_leaf {
                    return Err(SubledgerError::AccountNotLeaf(line.account_no.clone()));
                }

                sqlx::query(
                    r#"
                    INSERT INTO journal_lines (entry_id, account_no, amount, currency)
                    VALUES ($1, $2, ($3)::numeric, $4)
                    "#,
                )
                .bind(entry_id)
                .bind(&line.account_no)
                .bind(line.amount.to_string())
                .bind(&line.currency)
                .execute(&mut **tx)
                .await?;

                total_lines += 1;
            }
        }

        Ok(BatchResult {
            batch_id,
            entry_count: input.entries.len(),
            total_lines,
        })
    }

    pub async fn get_policy_batches(
        &self,
        policy_id: Uuid,
    ) -> Result<Vec<BatchSummary>, SubledgerError> {
        let rows = sqlx::query(
            r#"
            SELECT
              b.batch_id,
              b.policy_id,
              b.event_type,
              b.created_at,
              COUNT(DISTINCT e.entry_id) AS entry_count,
              COUNT(l.line_id) AS line_count
            FROM accounting_batches b
            LEFT JOIN journal_entries e ON e.batch_id = b.batch_id
            LEFT JOIN journal_lines l ON l.entry_id = e.entry_id
            WHERE b.policy_id = $1
            GROUP BY b.batch_id, b.policy_id, b.event_type, b.created_at
            ORDER BY b.created_at DESC
            "#,
        )
        .bind(policy_id)
        .fetch_all(&**self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(BatchSummary {
                batch_id: row.try_get("batch_id")?,
                policy_id: row.try_get("policy_id")?,
                event_type: row.try_get("event_type")?,
                created_at: row.try_get("created_at")?,
                entry_count: row.try_get::<i64, _>("entry_count")? as usize,
                line_count: row.try_get::<i64, _>("line_count")? as usize,
            });
        }
        Ok(out)
    }

    pub async fn get_account_balance(
        &self,
        account_no: &str,
        as_of_date: NaiveDate,
    ) -> Result<AccountBalance, SubledgerError> {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM ledger_accounts WHERE account_no = $1 LIMIT 1",
        )
        .bind(account_no)
        .fetch_optional(&**self.pool)
        .await?;
        if exists.is_none() {
            return Err(SubledgerError::AccountNotFound(account_no.to_string()));
        }

        let balance_text: String = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(l.amount), 0)::text AS balance
            FROM journal_lines l
            JOIN journal_entries e ON e.entry_id = l.entry_id
            WHERE l.account_no = $1
              AND e.effective_date <= $2
            "#,
        )
        .bind(account_no)
        .bind(as_of_date)
        .fetch_one(&**self.pool)
        .await?;

        let balance = Decimal::from_str(&balance_text)
            .map_err(|e| SubledgerError::DatabaseError(format!("invalid numeric balance: {e}")))?;

        Ok(AccountBalance {
            account_no: account_no.to_string(),
            as_of_date,
            balance,
        })
    }

    pub async fn verify_batch_balance(&self, batch_id: Uuid) -> Result<bool, SubledgerError> {
        let net_text: String = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(l.amount), 0)::text AS net
            FROM journal_lines l
            JOIN journal_entries e ON e.entry_id = l.entry_id
            WHERE e.batch_id = $1
            "#,
        )
        .bind(batch_id)
        .fetch_one(&**self.pool)
        .await?;

        let net = Decimal::from_str(&net_text)
            .map_err(|e| SubledgerError::DatabaseError(format!("invalid numeric net: {e}")))?;

        Ok(net == Decimal::ZERO)
    }
}

async fn account_leaf_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_no: &str,
) -> Result<bool, SubledgerError> {
    let row = sqlx::query("SELECT is_leaf FROM ledger_accounts WHERE account_no = $1")
        .bind(account_no)
        .fetch_optional(&mut **tx)
        .await?;

    let Some(row) = row else {
        return Err(SubledgerError::AccountNotFound(account_no.to_string()));
    };

    let is_leaf: bool = row.try_get("is_leaf")?;
    Ok(is_leaf)
}

fn validate_entry_balance(
    entry_index: usize,
    entry: &JournalEntryInput,
) -> Result<(), SubledgerError> {
    let net = entry
        .lines
        .iter()
        .fold(Decimal::ZERO, |acc, line| acc + line.amount);
    if net != Decimal::ZERO {
        return Err(SubledgerError::UnbalancedEntry { entry_index, net });
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInput {
    pub policy_id: Uuid,
    pub event_type: String,
    pub entries: Vec<JournalEntryInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntryInput {
    pub effective_date: NaiveDate,
    pub lines: Vec<JournalLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalLineInput {
    pub account_no: String,
    pub amount: Decimal,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub batch_id: Uuid,
    pub entry_count: usize,
    pub total_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSummary {
    pub batch_id: Uuid,
    pub policy_id: Uuid,
    pub event_type: String,
    pub created_at: DateTime<Utc>,
    pub entry_count: usize,
    pub line_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalance {
    pub account_no: String,
    pub as_of_date: NaiveDate,
    pub balance: Decimal,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PremiumLedgerError {
    #[error("policy was not found")]
    PolicyNotFound,
    #[error("premium ledger entry sequence overflow")]
    SequenceOverflow,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SubledgerError {
    #[error("database error: {0}")]
    DatabaseError(String),
    #[error("entry {entry_index} is unbalanced with net={net}")]
    UnbalancedEntry { entry_index: usize, net: Decimal },
    #[error("account not found: {0}")]
    AccountNotFound(String),
    #[error("account is not a leaf posting account: {0}")]
    AccountNotLeaf(String),
}

impl From<sqlx::Error> for SubledgerError {
    fn from(value: sqlx::Error) -> Self {
        Self::DatabaseError(value.to_string())
    }
}

fn sort_entries(entries: &mut [PremiumEntry]) {
    entries.sort_by_key(|entry| (entry.effective_at, entry.sequence));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn dec(v: &str) -> Decimal {
        Decimal::from_str(v).unwrap()
    }

    fn net_amount_cents(entries: &[PremiumEntry]) -> i64 {
        entries
            .iter()
            .map(|entry| match &entry.kind {
                PremiumEntryKind::WrittenPremium => 0,
                PremiumEntryKind::PremiumAdjustment { delta_cents } => *delta_cents,
                PremiumEntryKind::EndorsementAdjustment { delta_cents, .. } => *delta_cents,
                PremiumEntryKind::CancellationRefund { amount_cents } => -*amount_cents,
                PremiumEntryKind::ReinstatementCharge { amount_cents } => *amount_cents,
                PremiumEntryKind::EarnedPremiumSnapshot { amount_cents } => *amount_cents,
            })
            .sum()
    }

    #[test]
    fn test_append_and_retrieve() {
        let ledger = PremiumLedger::new();
        let policy_id = PolicyId::new();

        ledger.append(
            policy_id,
            PremiumEntryKind::WrittenPremium,
            ts(2026, 1, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::CancellationRefund {
                amount_cents: 50_000,
            },
            ts(2026, 3, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::EndorsementAdjustment {
                endorsement_id: EndorsementId::new(),
                delta_cents: 25_000,
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
        assert!(matches!(entries[0].kind, PremiumEntryKind::WrittenPremium));
        assert!(matches!(
            entries[1].kind,
            PremiumEntryKind::EndorsementAdjustment { .. }
        ));
        assert!(matches!(
            entries[2].kind,
            PremiumEntryKind::CancellationRefund { .. }
        ));
    }

    #[test]
    fn test_adjustment_flow() {
        let ledger = PremiumLedger::new();
        let policy_id = PolicyId::new();
        let endorsement_id = EndorsementId::new();

        ledger.append(
            policy_id,
            PremiumEntryKind::WrittenPremium,
            ts(2026, 1, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::PremiumAdjustment {
                delta_cents: 245_000,
            },
            ts(2026, 1, 1),
            "pricing",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::EndorsementAdjustment {
                endorsement_id,
                delta_cents: 50_000,
            },
            ts(2026, 2, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::CancellationRefund {
                amount_cents: 100_000,
            },
            ts(2026, 3, 1),
            "rust-pas",
        );

        let entries = ledger.entries(&policy_id);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[1].sequence, 2);
        assert_eq!(entries[2].sequence, 3);
        assert_eq!(entries[3].sequence, 4);
        assert_eq!(net_amount_cents(&entries), 195_000);
    }

    #[test]
    fn test_as_of_snapshot() {
        let ledger = PremiumLedger::new();
        let policy_id = PolicyId::new();

        ledger.append(
            policy_id,
            PremiumEntryKind::WrittenPremium,
            ts(2026, 1, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::PremiumAdjustment {
                delta_cents: 245_000,
            },
            ts(2026, 1, 1),
            "pricing",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::EndorsementAdjustment {
                endorsement_id: EndorsementId::new(),
                delta_cents: 50_000,
            },
            ts(2026, 2, 1),
            "rust-pas",
        );
        ledger.append(
            policy_id,
            PremiumEntryKind::CancellationRefund {
                amount_cents: 100_000,
            },
            ts(2026, 3, 1),
            "rust-pas",
        );

        let jan_31 = ledger.as_of(&policy_id, ts(2026, 1, 31));
        assert_eq!(jan_31.len(), 2);
        assert_eq!(net_amount_cents(&jan_31), 245_000);

        let feb_15 = ledger.as_of(&policy_id, ts(2026, 2, 15));
        assert_eq!(feb_15.len(), 3);
        assert_eq!(net_amount_cents(&feb_15), 295_000);

        let mar_15 = ledger.as_of(&policy_id, ts(2026, 3, 15));
        assert_eq!(mar_15.len(), 4);
        assert_eq!(net_amount_cents(&mar_15), 195_000);
    }

    #[test]
    fn test_balanced_entry_passes() {
        let entry = JournalEntryInput {
            effective_date: date(2026, 1, 1),
            lines: vec![
                JournalLineInput {
                    account_no: "1210".into(),
                    amount: dec("1250.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2100".into(),
                    amount: dec("-1200.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2200".into(),
                    amount: dec("-50.00"),
                    currency: "USD".into(),
                },
            ],
        };

        assert!(validate_entry_balance(0, &entry).is_ok());
    }

    #[test]
    fn test_unbalanced_entry_fails() {
        let entry = JournalEntryInput {
            effective_date: date(2026, 1, 1),
            lines: vec![
                JournalLineInput {
                    account_no: "1210".into(),
                    amount: dec("100.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2100".into(),
                    amount: dec("-99.99"),
                    currency: "USD".into(),
                },
            ],
        };

        let err = validate_entry_balance(2, &entry).unwrap_err();
        assert_eq!(
            err,
            SubledgerError::UnbalancedEntry {
                entry_index: 2,
                net: dec("0.01")
            }
        );
    }

    #[test]
    fn test_bind_new_business_scenario() {
        let entry = JournalEntryInput {
            effective_date: date(2026, 1, 1),
            lines: vec![
                JournalLineInput {
                    account_no: "1210".into(),
                    amount: dec("1250.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2100".into(),
                    amount: dec("-1200.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2200".into(),
                    amount: dec("-120.00"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "4010".into(),
                    amount: dec("70.00"),
                    currency: "USD".into(),
                },
            ],
        };

        assert!(validate_entry_balance(0, &entry).is_ok());
    }

    #[test]
    fn test_pro_rata_cancellation_scenario() {
        let entry = JournalEntryInput {
            effective_date: date(2026, 7, 2),
            lines: vec![
                JournalLineInput {
                    account_no: "2100".into(),
                    amount: dec("598.36"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2200".into(),
                    amount: dec("59.84"),
                    currency: "USD".into(),
                },
                JournalLineInput {
                    account_no: "2310".into(),
                    amount: dec("-658.20"),
                    currency: "USD".into(),
                },
            ],
        };

        assert!(validate_entry_balance(0, &entry).is_ok());
    }
}
