-- Immutable FNOL lifecycle events

CREATE TABLE IF NOT EXISTS fnol_events (
    id BIGSERIAL PRIMARY KEY,
    event_type TEXT NOT NULL,
    process_id UUID NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    payload JSONB NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_fnol_events_process_occurred
    ON fnol_events(process_id, occurred_at);

CREATE INDEX IF NOT EXISTS idx_fnol_events_type_occurred
    ON fnol_events(event_type, occurred_at);
