-- Per-stage pipeline output (doc-pipeline's PipelineResult.stages), so a
-- submission's stages are independently inspectable via GET .../trace
-- instead of only the final extracted_json/summary_json/analysis_json blob.
-- Satisfies the consolidation decision's traceability requirement: "every
-- pipeline stage individually visible and inspectable."

ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS stages_json JSONB NULL;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS stages_json JSONB NULL;

-- uw_jobs had `ticket_id` only (an artifact of before the ticket-vs-deal
-- split was made explicit). Underwriting creates a HubSpot *deal*, not a
-- ticket -- "the two HubSpot cards stay two cards" means these need their
-- own column, not `ticket_id` doing double duty for two different object
-- types depending on which domain wrote the row.
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS deal_id TEXT NULL;
CREATE INDEX IF NOT EXISTS idx_uw_jobs_deal ON uw_jobs(deal_id);
