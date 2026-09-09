#!/usr/bin/env python3
"""Regenerates seed-uw-demo.sql from uw-demo-source.json.

uw-demo-source.json is real extracted output from a real completed job
(job-c70f11524c6746d08a3f7e2db963cb11), sourced from the NVMe cosmos
backup of the old standalone UW workbench -- not fabricated. This script
reshapes it into sagepas's own uw_jobs schema: analysis_json fields become
citation leaves ({value, confidence, page_number}) matching what
CitedFieldTree renders, grouped by the real page each field came from.

confidence is honestly null throughout -- the source system never
produced a per-field numeric confidence, and this doesn't invent one.
status is 'review_required', not 'processed' -- the real outcome was an
incomplete submission that correctly got flagged for more documents, not
a finished pass.

Run: python3 generate-seed-uw-demo.py > seed-uw-demo.sql
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))

with open(os.path.join(HERE, "uw-demo-source.json")) as f:
    raw = json.load(f)


def sql_str(s):
    return "'" + s.replace("'", "''") + "'"


pages_by_num = {p["page_number"]: p for p in raw["pages"]}


def cite_fields(key_values, page):
    out = {}
    for k, v in key_values.items():
        if v in (None, ""):
            continue
        out[k] = {"value": v, "confidence": None, "page_number": page}
    return out


total_fields = sum(len(p.get("key_values", {})) for p in raw["pages"])

analysis_json = {
    "agency_and_applicant": cite_fields(pages_by_num.get(1, {}).get("key_values", {}), 1),
    "location_1": cite_fields(pages_by_num.get(2, {}).get("key_values", {}), 2),
    "location_2": cite_fields(pages_by_num.get(3, {}).get("key_values", {}), 3),
    "coverage_schedule": cite_fields(pages_by_num.get(4, {}).get("key_values", {}), 4),
    "crime_report": cite_fields(pages_by_num.get(5, {}).get("key_values", {}), 5),
    "kb_findings": None,
    "_cited_field_count": total_fields,
    "_scored_field_count": total_fields,
}

stage_detail = {
    "CLASSIFYING": {"document_type": raw["documentType"], "insurance_type": raw["insuranceType"]},
    "EXTRACTING": {"pages_extracted": len(raw["pages"]), "filename": raw["filename"]},
    "DETECTING": raw["detection"],
    "ANALYZING": {"summary": raw["analysisSummary"]},
    "SCORING": raw["scoring"],
    "ACTING": raw["action"],
}
stages_json = [
    {"name": n, "status": "complete", "detail": stage_detail[n]}
    for n in ["CLASSIFYING", "EXTRACTING", "DETECTING", "ANALYZING", "SCORING", "ACTING"]
]

analysis_str = json.dumps(analysis_json)
stages_str = json.dumps(stages_json)

print(f"""
INSERT INTO uw_jobs (
  job_id, insurance_type, status, analysis_json, stages_json,
  original_filename, mime_type, blob_container, blob_name, recommendation
) VALUES (
  {sql_str(raw['jobId'])},
  {sql_str(raw['insuranceType'])},
  'review_required',
  $analysis${analysis_str}$analysis$::jsonb,
  $stages${stages_str}$stages$::jsonb,
  {sql_str(raw['filename'])},
  'application/pdf',
  'uw-documents',
  {sql_str(raw['filename'])},
  NULL
)
ON CONFLICT (job_id) DO UPDATE SET
  analysis_json = EXCLUDED.analysis_json,
  stages_json = EXCLUDED.stages_json,
  status = EXCLUDED.status;
""")
