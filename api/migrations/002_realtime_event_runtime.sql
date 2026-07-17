-- Realtime/event-runtime support tables

CREATE TABLE IF NOT EXISTS processed_events (
    event_id UUID PRIMARY KEY,
    event_type TEXT NOT NULL,
    consumer TEXT NOT NULL DEFAULT 'event-stream-runtime',
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS projection_checkpoints (
    name TEXT PRIMARY KEY,
    last_event_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_processed_events_processed_at
    ON processed_events (processed_at DESC);
