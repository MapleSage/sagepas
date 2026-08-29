-- Evaluate stage now produces two independent numbers (entity_score,
-- schema_score) instead of one null-density figure. The existing
-- `confidence` column keeps its name and is now populated with
-- entity_score (the real "was the text read correctly" signal, not a
-- rename in the DB -- callers/frontend already read `confidence`).
-- schema_score is additive: a new column, not a replacement.
ALTER TABLE fnol_submissions ADD COLUMN IF NOT EXISTS schema_score DOUBLE PRECISION NULL;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS schema_score DOUBLE PRECISION NULL;
