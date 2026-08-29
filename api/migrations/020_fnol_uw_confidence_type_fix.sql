-- confidence (both tables) and fnol_submissions.schema_score were created
-- as NUMERIC by an earlier migration; every Rust caller (FnolListRow,
-- FnolTraceRow, UwListRow, UwTraceRow, the UPDATE ... bind(f64) calls)
-- has always used f64/DOUBLE PRECISION. This was latent -- it only surfaced
-- once a real submission finally completed end-to-end and a SELECT with a
-- populated row actually ran sqlx's decode path. uw_jobs.schema_score was
-- created fresh by migration 017 as DOUBLE PRECISION already, so it's
-- unaffected here.
ALTER TABLE fnol_submissions ALTER COLUMN confidence TYPE DOUBLE PRECISION USING confidence::double precision;
ALTER TABLE fnol_submissions ALTER COLUMN schema_score TYPE DOUBLE PRECISION USING schema_score::double precision;
ALTER TABLE uw_jobs ALTER COLUMN confidence TYPE DOUBLE PRECISION USING confidence::double precision;
