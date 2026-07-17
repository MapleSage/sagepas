use crate::ids::{AgentId, CustomerId, PolicyId, ProductId, QuoteId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Government-issued national identity document type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NationalIdType {
    /// US Social Security Number
    Ssn,
    /// US Individual Taxpayer Identification Number
    Itin,
    /// India Aadhaar card
    Aadhar,
    /// India Permanent Account Number
    Pan,
    Other,
}

/// Broad line of insurance business.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsuranceType {
    Auto,
    Life,
    Health,
    Property,
    Marine,
}

/// ISO 4217 currency codes in use on the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Currency {
    Usd,
    Inr,
}

/// Lifecycle state of a quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteState {
    QuickQuote,
    Quoted,
    Bound,
    Issued,
    Expired,
    Cancelled,
}

/// Lifecycle state of a policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyState {
    Active,
    Expired,
    Cancelled,
}

/// Policyholder / insured individual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: CustomerId,
    pub user_id: UserId,
    pub name: String,
    pub email: String,
    pub phone: String,
    /// ISO 3166-1 alpha-2 country code: "US" or "IN".
    pub country: String,
    pub currency: Currency,
    pub national_id: Option<String>,
    pub national_id_type: Option<NationalIdType>,
    pub address: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// An insurance product offered on the platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: ProductId,
    pub name: String,
    pub insurance_type: InsuranceType,
    pub country: String,
    pub currency: Currency,
    pub description: Option<String>,
}

/// A single multiplicative adjustment applied during premium calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateFactor {
    pub name: String,
    pub value: f64,
    pub description: Option<String>,
}

/// Full breakdown of a premium computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PremiumCalculation {
    pub base_rate: f64,
    pub factors: Vec<RateFactor>,
    pub final_premium: f64,
    pub currency: Currency,
}

/// A pricing quote issued to a customer before binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub id: QuoteId,
    pub customer_id: CustomerId,
    pub product_id: ProductId,
    pub state: QuoteState,
    pub premium: f64,
    pub currency: Currency,
    /// Flexible coverage details stored as a JSON object.
    pub coverage: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A bound and issued insurance policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: PolicyId,
    pub quote_id: QuoteId,
    pub policy_number: String,
    pub customer_id: CustomerId,
    pub product_id: ProductId,
    pub state: PolicyState,
    pub premium: f64,
    pub currency: Currency,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    /// Azure Blob URL to the generated PDF declaration page.
    pub pdf_url: Option<String>,
    /// Coverage details matching the quote, stored as a JSON object.
    pub coverage: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A licensed insurance agent who may earn commissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub email: String,
    /// Commission percentage (e.g. 5.0 = 5 %).
    pub commission_rate_pct: f64,
    pub country: String,
    pub created_at: DateTime<Utc>,
}
