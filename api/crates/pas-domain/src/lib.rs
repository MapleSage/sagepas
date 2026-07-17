pub use domain::ids::{AgentId, CustomerId, PolicyId, QuoteId, UserId};
pub type BindId = PolicyId;
pub type EndorsementId = UserId;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PolicyNumber(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QuoteNumber(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClaimNumber(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountNumber(String);

impl PolicyNumber {
    pub fn new(value: impl Into<String>) -> Result<Self, PasValidationError> {
        let value = value.into();
        validate_business_number("POL", &value)?;
        Ok(Self(value))
    }

    pub fn deterministic(seed: &str) -> Self {
        Self(format_business_number("POL", seed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PolicyNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for PolicyNumber {
    type Err = PasValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl QuoteNumber {
    pub fn new(value: impl Into<String>) -> Result<Self, PasValidationError> {
        let value = value.into();
        validate_business_number("QUO", &value)?;
        Ok(Self(value))
    }

    pub fn deterministic(seed: &str) -> Self {
        Self(format_business_number("QUO", seed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for QuoteNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for QuoteNumber {
    type Err = PasValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl ClaimNumber {
    pub fn new(value: impl Into<String>) -> Result<Self, PasValidationError> {
        let value = value.into();
        validate_business_number("CLM", &value)?;
        Ok(Self(value))
    }

    pub fn deterministic(seed: &str) -> Self {
        Self(format_business_number("CLM", seed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClaimNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ClaimNumber {
    type Err = PasValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl AccountNumber {
    pub fn new(value: impl Into<String>) -> Result<Self, PasValidationError> {
        let value = value.into();
        validate_business_number("ACC", &value)?;
        Ok(Self(value))
    }

    pub fn deterministic(seed: &str) -> Self {
        Self(format_business_number("ACC", seed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for AccountNumber {
    type Err = PasValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineOfBusiness {
    Homeowners,
    DwellingFire,
    Auto,
    CommercialProperty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Requested,
    Quoted,
    Bound,
    Issued,
    Suspended,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndorsementStatus {
    Draft,
    Quoted,
    Applied,
    Declined,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    NotRequired,
    Pending,
    Authorized,
    Captured,
    Failed,
    Refunded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStatus {
    Draft,
    Rated,
    Offered,
    Bound,
    Expired,
    Declined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    PendingIssue,
    Active,
    Endorsed,
    Cancelled,
    Expired,
    Reinstated,
}

impl QuoteStatus {
    pub fn can_transition_to(&self, next: &Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Rated)
                | (Self::Rated, Self::Offered)
                | (Self::Offered, Self::Bound)
                | (Self::Offered, Self::Expired)
                | (Self::Offered, Self::Declined)
        )
    }
}

impl PolicyStatus {
    pub fn can_transition_to(&self, next: &Self) -> bool {
        matches!(
            (self, next),
            (Self::PendingIssue, Self::Active)
                | (Self::Active, Self::Endorsed)
                | (Self::Endorsed, Self::Active)
                | (Self::Active, Self::Cancelled)
                | (Self::Endorsed, Self::Cancelled)
                | (Self::Cancelled, Self::Reinstated)
                | (Self::Reinstated, Self::Active)
                | (Self::Active, Self::Expired)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: Option<UserId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasResponse<T> {
    pub request_id: UserId,
    pub source: String,
    pub data: T,
}

impl<T> PasResponse<T> {
    pub fn rust_native(request_id: UserId, data: T) -> Self {
        Self {
            request_id,
            source: "rust-pas".to_string(),
            data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageSelection {
    pub code: String,
    pub limit: u64,
    pub deductible: u64,
    pub status: CoverageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerRef {
    pub customer_id: CustomerId,
    pub account_number: AccountNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteInput {
    pub customer: CustomerRef,
    pub line_of_business: LineOfBusiness,
    pub state: String,
    pub effective_date: NaiveDate,
    pub coverages: Vec<CoverageSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindInput {
    pub quote_id: QuoteId,
    pub account_number: AccountNumber,
    pub payment_status: PaymentStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteResponse {
    pub quote_id: QuoteId,
    pub quote_number: QuoteNumber,
    pub customer_id: CustomerId,
    pub account_number: AccountNumber,
    pub status: QuoteStatus,
    pub line_of_business: LineOfBusiness,
    pub annual_premium_cents: u64,
    pub currency: String,
    pub coverages: Vec<CoverageSelection>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindResponse {
    pub quote_id: QuoteId,
    pub quote_number: QuoteNumber,
    pub policy_id: PolicyId,
    pub policy_number: PolicyNumber,
    pub account_number: AccountNumber,
    pub quote_status: QuoteStatus,
    pub policy_status: PolicyStatus,
    pub payment_status: PaymentStatus,
    pub annual_premium_cents: u64,
    pub currency: String,
    pub bound_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyResponse {
    pub policy_id: PolicyId,
    pub policy_number: PolicyNumber,
    pub account_id: AccountNumber,
    pub status: PolicyStatus,
    pub effective_date: NaiveDate,
    pub coverages: Vec<CoverageSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndorsementInput {
    pub coverages: Vec<CoverageSelection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndorsementResponse {
    pub endorsement_id: EndorsementId,
    pub policy_id: PolicyId,
    pub status: EndorsementStatus,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PasValidationError {
    #[error("{field} is required")]
    MissingField { field: &'static str },
    #[error("{field} is invalid: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
}

impl QuoteInput {
    pub fn validate(&self) -> Result<(), PasValidationError> {
        if self.state.trim().len() != 2 {
            return Err(PasValidationError::InvalidField {
                field: "state",
                reason: "must be a two-character US state code".to_string(),
            });
        }
        if self.coverages.is_empty() {
            return Err(PasValidationError::MissingField { field: "coverages" });
        }
        if self.coverages.iter().any(|coverage| coverage.limit == 0) {
            return Err(PasValidationError::InvalidField {
                field: "coverages.limit",
                reason: "must be greater than zero".to_string(),
            });
        }
        Ok(())
    }
}

impl BindInput {
    pub fn validate(&self) -> Result<(), PasValidationError> {
        if matches!(
            self.payment_status,
            PaymentStatus::Failed | PaymentStatus::Refunded
        ) {
            return Err(PasValidationError::InvalidField {
                field: "payment_status",
                reason: "cannot bind with failed or refunded payment".to_string(),
            });
        }
        Ok(())
    }
}

impl EndorsementInput {
    pub fn validate(&self) -> Result<(), PasValidationError> {
        if self.coverages.is_empty() {
            return Err(PasValidationError::MissingField { field: "coverages" });
        }
        if self.coverages.iter().any(|coverage| coverage.limit == 0) {
            return Err(PasValidationError::InvalidField {
                field: "coverages.limit",
                reason: "must be greater than zero".to_string(),
            });
        }
        Ok(())
    }
}

fn validate_business_number(prefix: &'static str, value: &str) -> Result<(), PasValidationError> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 || parts[0] != prefix {
        return Err(PasValidationError::InvalidField {
            field: "number",
            reason: format!("must match {prefix}-YYYY-NNNNNN"),
        });
    }

    let year = parts[1];
    let sequence = parts[2];
    if year.len() != 4
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || sequence.len() != 6
        || !sequence.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(PasValidationError::InvalidField {
            field: "number",
            reason: format!("must match {prefix}-YYYY-NNNNNN"),
        });
    }

    Ok(())
}

fn format_business_number(prefix: &str, seed: &str) -> String {
    format!("{prefix}-2026-{:06}", stable_number(seed))
}

fn stable_number(seed: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in seed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash % 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer_ref() -> CustomerRef {
        CustomerRef {
            customer_id: CustomerId::new(),
            account_number: AccountNumber::deterministic("account-a"),
        }
    }

    #[test]
    fn deterministic_numbers_are_prefixed_and_stable() {
        let one = PolicyNumber::deterministic("quote-123");
        let two = PolicyNumber::deterministic("quote-123");
        assert_eq!(one, two);
        assert!(one.as_str().starts_with("POL-2026-"));
        assert_eq!(
            QuoteNumber::new("QUO-2026-000123").unwrap().as_str(),
            "QUO-2026-000123"
        );
        assert!(ClaimNumber::new("CLM-2026-ABC123").is_err());
    }

    #[test]
    fn validates_quote_input() {
        let input = QuoteInput {
            customer: customer_ref(),
            line_of_business: LineOfBusiness::Homeowners,
            state: "TX".to_string(),
            effective_date: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            coverages: vec![CoverageSelection {
                code: "COV_A".to_string(),
                limit: 300_000,
                deductible: 2_500,
                status: CoverageStatus::Requested,
            }],
        };

        assert_eq!(input.validate(), Ok(()));
    }

    #[test]
    fn rejects_failed_payment_bind() {
        let input = BindInput {
            quote_id: QuoteId::new(),
            account_number: AccountNumber::deterministic("account-a"),
            payment_status: PaymentStatus::Failed,
        };

        assert!(matches!(
            input.validate(),
            Err(PasValidationError::InvalidField {
                field: "payment_status",
                ..
            })
        ));
    }

    #[test]
    fn exposes_lifecycle_transition_rules() {
        assert!(QuoteStatus::Offered.can_transition_to(&QuoteStatus::Bound));
        assert!(!QuoteStatus::Draft.can_transition_to(&QuoteStatus::Bound));
        assert!(PolicyStatus::PendingIssue.can_transition_to(&PolicyStatus::Active));
        assert!(!PolicyStatus::Expired.can_transition_to(&PolicyStatus::Active));
    }
}
