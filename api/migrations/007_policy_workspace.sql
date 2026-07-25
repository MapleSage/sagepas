-- Policy workspace records: payments, claims, notifications, and renewals.
-- These tables back the user-facing policy tabs; they are not sample/UI-only data.

CREATE TABLE IF NOT EXISTS policy_payments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    amount          DOUBLE PRECISION NOT NULL CHECK (amount > 0),
    currency        CHAR(3) NOT NULL,
    payment_method  TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'received',
    reference       TEXT,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_policy_payments_policy ON policy_payments(policy_id, created_at DESC);

CREATE TABLE IF NOT EXISTS policy_claims (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    claim_number    TEXT NOT NULL UNIQUE,
    loss_date       DATE NOT NULL,
    loss_type       TEXT NOT NULL,
    description     TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'reported',
    reserve_amount  DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (reserve_amount >= 0),
    currency        CHAR(3) NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_policy_claims_policy ON policy_claims(policy_id, created_at DESC);

CREATE TABLE IF NOT EXISTS policy_notifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    category        TEXT NOT NULL,
    subject         TEXT NOT NULL,
    message         TEXT NOT NULL,
    channel         TEXT NOT NULL DEFAULT 'in_app',
    status          TEXT NOT NULL DEFAULT 'recorded',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_policy_notifications_policy ON policy_notifications(policy_id, created_at DESC);

CREATE TABLE IF NOT EXISTS policy_renewals (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    renewal_quote_id UUID NOT NULL REFERENCES quotes(id),
    effective_date  DATE NOT NULL,
    status          TEXT NOT NULL DEFAULT 'quoted',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_policy_renewals_policy ON policy_renewals(policy_id, created_at DESC);

-- UAE/AED products are first-class catalog records, not a display-only currency option.
INSERT INTO products (id, name, insurance_type, country, currency, description)
SELECT gen_random_uuid(), v.name, v.insurance_type, 'AE', 'AED', v.description
FROM (VALUES
    ('Auto Insurance UAE', 'auto', 'Motor insurance for UAE drivers'),
    ('Property Insurance UAE', 'property', 'Property insurance for UAE homes and businesses')
) AS v(name, insurance_type, description)
WHERE NOT EXISTS (
    SELECT 1 FROM products p WHERE p.name = v.name AND p.country = 'AE' AND p.currency = 'AED'
);
