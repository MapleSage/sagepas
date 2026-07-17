-- Durable quote timeline read model for event-stream-runtime

CREATE TABLE IF NOT EXISTS quote_timeline_projections (
    quote_id UUID PRIMARY KEY,
    policy_id UUID,
    customer_id UUID NOT NULL,
    product_id UUID NOT NULL,
    premium DOUBLE PRECISION,
    currency TEXT,
    projected_stage TEXT,
    projected_event_count INTEGER NOT NULL DEFAULT 0,
    timeline JSONB NOT NULL DEFAULT '[]'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quote_timeline_projections_updated_at
    ON quote_timeline_projections (updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_quote_timeline_projections_stage
    ON quote_timeline_projections (projected_stage);
