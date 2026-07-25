-- Insurance platform schema migration 001
-- Creates all tables for the SageSure US/India insurance platform.

CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'consumer',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS customers (
    id              UUID PRIMARY KEY,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    email           TEXT NOT NULL,
    phone           TEXT NOT NULL,
    country         TEXT NOT NULL DEFAULT 'US',
    currency        TEXT NOT NULL DEFAULT 'USD',
    national_id     TEXT,
    national_id_type TEXT,
    address         TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS products (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    insurance_type  TEXT NOT NULL,
    country         TEXT NOT NULL,
    currency        TEXT NOT NULL,
    description     TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quotes (
    id          UUID PRIMARY KEY,
    customer_id UUID NOT NULL REFERENCES customers(id),
    product_id  UUID NOT NULL REFERENCES products(id),
    state       TEXT NOT NULL DEFAULT 'quick_quote',
    premium     DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency    TEXT NOT NULL DEFAULT 'USD',
    coverage    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS policies (
    id            UUID PRIMARY KEY,
    quote_id      UUID NOT NULL REFERENCES quotes(id),
    policy_number TEXT NOT NULL UNIQUE,
    customer_id   UUID NOT NULL REFERENCES customers(id),
    product_id    UUID NOT NULL REFERENCES products(id),
    state         TEXT NOT NULL DEFAULT 'active',
    premium       DOUBLE PRECISION NOT NULL,
    currency      TEXT NOT NULL DEFAULT 'USD',
    start_date    TIMESTAMPTZ NOT NULL,
    end_date      TIMESTAMPTZ NOT NULL,
    pdf_url       TEXT,
    coverage      JSONB NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS claims (
    id              UUID PRIMARY KEY,
    policy_id       UUID NOT NULL REFERENCES policies(id),
    customer_id     UUID NOT NULL REFERENCES customers(id),
    fnol_process_id TEXT,
    state           TEXT NOT NULL DEFAULT 'filed',
    amount          DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency        TEXT NOT NULL DEFAULT 'USD',
    description     TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS agents (
    id                  UUID PRIMARY KEY,
    name                TEXT NOT NULL,
    email               TEXT NOT NULL UNIQUE,
    commission_rate_pct DOUBLE PRECISION NOT NULL DEFAULT 5.0,
    country             TEXT NOT NULL DEFAULT 'US',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS commissions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id    UUID NOT NULL REFERENCES agents(id),
    policy_id   UUID NOT NULL REFERENCES policies(id),
    amount      DOUBLE PRECISION NOT NULL,
    currency    TEXT NOT NULL DEFAULT 'USD',
    paid_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed default products (idempotent)
INSERT INTO products (id, name, insurance_type, country, currency, description) VALUES
    (gen_random_uuid(), 'Auto Insurance',       'auto', 'US', 'USD', 'Personal auto insurance for US drivers'),
    (gen_random_uuid(), 'Life Insurance',        'life', 'US', 'USD', 'Term life insurance'),
    (gen_random_uuid(), 'Auto Insurance India',  'auto', 'IN', 'INR', 'Motor insurance for India'),
    (gen_random_uuid(), 'Life Insurance India',  'life', 'IN', 'INR', 'Term life insurance India')
ON CONFLICT DO NOTHING;
