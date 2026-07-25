CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS conversation_sessions (
    session_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL,
    surface         TEXT NOT NULL DEFAULT 'chat',
    workflow_id     TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata        JSONB NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS conversation_messages (
    message_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      UUID NOT NULL REFERENCES conversation_sessions(session_id),
    role            TEXT NOT NULL CHECK (role IN ('user','assistant','system')),
    content         TEXT NOT NULL,
    surface         TEXT NOT NULL DEFAULT 'chat',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS conversation_memory_facts (
    fact_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL,
    session_id      UUID,
    fact_type       TEXT NOT NULL,
    fact_key        TEXT NOT NULL,
    fact_value      TEXT NOT NULL,
    confidence      NUMERIC(3,2) NOT NULL DEFAULT 1.00,
    source_surface  TEXT NOT NULL DEFAULT 'chat',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, fact_type, fact_key)
);

CREATE TABLE IF NOT EXISTS conversation_summaries (
    summary_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      UUID NOT NULL REFERENCES conversation_sessions(session_id),
    summary_text    TEXT NOT NULL,
    message_count   INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_conv_sessions_user ON conversation_sessions(user_id, last_active_at DESC);
CREATE INDEX IF NOT EXISTS idx_conv_messages_session ON conversation_messages(session_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_conv_facts_user ON conversation_memory_facts(user_id, fact_type, fact_key);
CREATE INDEX IF NOT EXISTS idx_conv_summaries_session ON conversation_summaries(session_id);
