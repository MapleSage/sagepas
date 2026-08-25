-- uw_jobs never got original_filename/mime_type when fnol_submissions did
-- (migration 010 added both to fnol_submissions only). Two real gaps this
-- closes: (1) the UW queue has been showing a raw job_id where FNOL shows
-- a filename -- part of the FNOL/UW UI inconsistency; (2) the document
-- viewer (Step 4) needs mime_type to serve the blob with the right
-- Content-Type, and had nothing to read for UW jobs.
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS original_filename TEXT NULL;
ALTER TABLE uw_jobs ADD COLUMN IF NOT EXISTS mime_type TEXT NULL;
