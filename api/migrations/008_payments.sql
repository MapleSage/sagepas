CREATE TABLE IF NOT EXISTS payments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES policies(id),
    customer_id     UUID NOT NULL REFERENCES customers(id),
    amount          NUMERIC(18,2) NOT NULL,
    currency        TEXT NOT NULL DEFAULT 'USD',
    payment_method  TEXT NOT NULL DEFAULT 'card',
    status          TEXT NOT NULL DEFAULT 'pending',
    reference       TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_payments_policy ON payments(policy_id);
CREATE INDEX IF NOT EXISTS idx_payments_customer ON payments(customer_id);
