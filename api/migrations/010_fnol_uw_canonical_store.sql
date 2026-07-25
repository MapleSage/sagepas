-- Native Rust FNOL/UW canonical store (Blob + Postgres metadata)

CREATE TABLE IF NOT EXISTS fnol_submissions (
    process_id UUID PRIMARY KEY,
    ticket_id TEXT NULL,
    blob_container TEXT NOT NULL DEFAULT 'fnol-documents',
    blob_name TEXT NOT NULL,
    original_filename TEXT NULL,
    mime_type TEXT NULL,
    status TEXT NOT NULL,
    extracted_json JSONB NULL,
    summary_json JSONB NULL,
    confidence NUMERIC NULL,
    decision TEXT NULL,
    decision_reason TEXT NULL,
    human_review_required BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS uw_jobs (
    job_id TEXT PRIMARY KEY,
    process_id UUID NULL REFERENCES fnol_submissions(process_id),
    ticket_id TEXT NULL,
    blob_container TEXT NOT NULL DEFAULT 'uw-documents',
    blob_name TEXT NOT NULL,
    status TEXT NOT NULL,
    analysis_json JSONB NULL,
    risk_score NUMERIC NULL,
    confidence NUMERIC NULL,
    recommendation TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS decision_audit_events (
    id BIGSERIAL PRIMARY KEY,
    process_id UUID NULL,
    job_id TEXT NULL,
    ticket_id TEXT NULL,
    stage TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    payload JSONB NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Upgrade older native-table shape in-place when 009_fnol_uw_native.sql already ran.
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS ticket_id TEXT NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS blob_container TEXT NOT NULL DEFAULT 'fnol-documents';
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS blob_name TEXT;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS original_filename TEXT NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS mime_type TEXT NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS summary_json JSONB NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS confidence NUMERIC NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS decision TEXT NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS decision_reason TEXT NULL;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS human_review_required BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE fnol_submissions SET blob_name = COALESCE(blob_name, blob_path) WHERE blob_name IS NULL;
ALTER TABLE fnol_submissions ALTER COLUMN blob_name SET NOT NULL;

ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS process_id UUID NULL REFERENCES fnol_submissions(process_id);
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS ticket_id TEXT NULL;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS blob_container TEXT NOT NULL DEFAULT 'uw-documents';
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS blob_name TEXT;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS recommendation TEXT NULL;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE uw_jobs SET blob_name = COALESCE(blob_name, blob_path) WHERE blob_name IS NULL;
UPDATE uw_jobs SET process_id = COALESCE(process_id, fnol_process_id) WHERE process_id IS NULL;
ALTER TABLE uw_jobs ALTER COLUMN blob_name SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_fnol_submissions_ticket ON fnol_submissions(ticket_id);
CREATE INDEX IF NOT EXISTS idx_fnol_submissions_status_updated ON fnol_submissions(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_uw_jobs_process ON uw_jobs(process_id);
CREATE INDEX IF NOT EXISTS idx_uw_jobs_ticket ON uw_jobs(ticket_id);
CREATE INDEX IF NOT EXISTS idx_uw_jobs_status_updated ON uw_jobs(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_decision_audit_process_created ON decision_audit_events(process_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_decision_audit_job_created ON decision_audit_events(job_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_decision_audit_ticket_created ON decision_audit_events(ticket_id, created_at DESC);
