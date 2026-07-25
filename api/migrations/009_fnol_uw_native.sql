CREATE TABLE IF NOT EXISTS fnol_submissions (
    process_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID REFERENCES policies(id),
    claim_id        UUID REFERENCES claims(id),
    blob_path       TEXT NOT NULL,
    schema_id       TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    entity_score    NUMERIC(5,4),
    schema_score    NUMERIC(5,4),
    extracted_json  JSONB,
    steps_json      JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS uw_jobs (
    job_id          TEXT PRIMARY KEY,
    fnol_process_id UUID REFERENCES fnol_submissions(process_id),
    blob_path       TEXT NOT NULL,
    insurance_type  TEXT NOT NULL DEFAULT 'commercial_property',
    status          TEXT NOT NULL DEFAULT 'pending',
    analysis_json   JSONB,
    risk_score      INTEGER,
    confidence      NUMERIC(3,2),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ
);


CREATE INDEX IF NOT EXISTS idx_fnol_submissions_claim ON fnol_submissions(claim_id);
CREATE INDEX IF NOT EXISTS idx_fnol_submissions_status ON fnol_submissions(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_uw_jobs_fnol ON uw_jobs(fnol_process_id);
CREATE INDEX IF NOT EXISTS idx_uw_jobs_status ON uw_jobs(status, created_at DESC);
