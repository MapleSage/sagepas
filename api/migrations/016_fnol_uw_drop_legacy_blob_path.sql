-- Migration 010 introduced blob_container/blob_name as the canonical blob
-- reference pair and backfilled them from blob_path, but left blob_path's
-- original NOT NULL constraint (from migration 009) in place. Nothing in
-- the Rust code reads blob_path any more (confirmed by grep across
-- api/crates and api/services) -- it's dead, but still NOT NULL, so every
-- INSERT into fnol_submissions/uw_jobs from the new doc-pipeline path
-- (which only sets blob_container/blob_name) fails.
ALTER TABLE fnol_submissions ALTER COLUMN blob_path DROP NOT NULL;
ALTER TABLE uw_jobs ALTER COLUMN blob_path DROP NOT NULL;
