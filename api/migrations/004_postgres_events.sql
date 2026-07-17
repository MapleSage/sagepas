-- Durable PostgreSQL-backed event store

CREATE TABLE IF NOT EXISTS postgres_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    stream_name TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sequence BIGINT NOT NULL GENERATED ALWAYS AS IDENTITY
);

CREATE INDEX IF NOT EXISTS idx_postgres_events_stream
    ON postgres_events(stream_name, sequence);

CREATE INDEX IF NOT EXISTS idx_postgres_events_type
    ON postgres_events(event_type, occurred_at);

CREATE INDEX IF NOT EXISTS idx_postgres_events_sequence
    ON postgres_events(sequence);
