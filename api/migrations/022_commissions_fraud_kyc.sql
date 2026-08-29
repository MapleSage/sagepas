-- Native Rust replacement for three sesure-us Django ERP modules that had
-- schema but no wired business logic: commissions, fraud, kyc.
-- Field/workflow shape taken from erp/{commissions,fraud,kyc}/models.py as
-- reference, not ported verbatim -- KYC in particular is adapted to US
-- identity documents (SSN/driver's license/passport) since the Django
-- version's pan_card/aadhar_number fields are India-market specific and
-- this is the US platform.

-- commissions: table already exists (001_insurance.sql) with
-- id/agent_id/policy_id/amount/currency/paid_at/created_at. Add the
-- workflow fields the Django reference has that are actually load-bearing
-- (rate the amount was computed from, and a status lifecycle) without
-- carrying over its separate CommissionPayment table -- paid_at already
-- covers "when it was paid" at this system's scale.
ALTER TABLE commissions ADD COLUMN IF NOT EXISTS commission_rate DOUBLE PRECISION;
ALTER TABLE commissions ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'approved', 'paid', 'reversed'));
CREATE INDEX IF NOT EXISTS idx_commissions_agent ON commissions(agent_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_commissions_policy ON commissions(policy_id);

-- fraud: risk scoring on claims. Reference: erp/fraud/models.py FraudRisk
-- (red-flag booleans -> weighted score -> level), simplified to the flags
-- that are actually computable from data already in this schema.
CREATE TABLE IF NOT EXISTS claim_fraud_risk (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    claim_id                UUID NOT NULL UNIQUE REFERENCES policy_claims(id) ON DELETE CASCADE,
    policy_id               UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    risk_score              INTEGER NOT NULL DEFAULT 0 CHECK (risk_score BETWEEN 0 AND 100),
    risk_level              TEXT NOT NULL DEFAULT 'low' CHECK (risk_level IN ('low', 'medium', 'high', 'critical')),
    status                  TEXT NOT NULL DEFAULT 'flagged'
        CHECK (status IN ('flagged', 'under_investigation', 'confirmed', 'cleared', 'closed')),
    is_duplicate_claim      BOOLEAN NOT NULL DEFAULT FALSE,
    is_over_claim           BOOLEAN NOT NULL DEFAULT FALSE,
    high_claim_frequency    BOOLEAN NOT NULL DEFAULT FALSE,
    description             TEXT NOT NULL DEFAULT '',
    investigation_notes     TEXT NOT NULL DEFAULT '',
    assigned_to             TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_claim_fraud_risk_policy ON claim_fraud_risk(policy_id);
CREATE INDEX IF NOT EXISTS idx_claim_fraud_risk_score ON claim_fraud_risk(risk_score DESC);

-- kyc: identity verification on customers. Reference: erp/kyc/models.py
-- KYCProfile, adapted to US identity documents.
CREATE TABLE IF NOT EXISTS kyc_profiles (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id                 UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    kyc_type                    TEXT NOT NULL DEFAULT 'individual'
        CHECK (kyc_type IN ('individual', 'corporate', 'partnership')),
    status                      TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'verified', 'rejected', 'expired')),
    identity_document_type      TEXT NOT NULL
        CHECK (identity_document_type IN ('ssn', 'drivers_license', 'passport', 'state_id')),
    identity_document_last4     TEXT NOT NULL,
    address_line1                TEXT NOT NULL,
    address_line2                TEXT NOT NULL DEFAULT '',
    city                         TEXT NOT NULL,
    state                        TEXT NOT NULL,
    postal_code                  TEXT NOT NULL,
    country                      TEXT NOT NULL DEFAULT 'US',
    verified_by                  TEXT,
    verified_at                  TIMESTAMPTZ,
    verification_notes           TEXT NOT NULL DEFAULT '',
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_kyc_profiles_customer ON kyc_profiles(customer_id);
