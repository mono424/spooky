-- Migration: add_job_delay
-- Created: 2026-06-24T16:46:17.148493+00:00

-- Added
DEFINE FIELD OVERWRITE assignee ON job TYPE none | string PERMISSIONS FOR select WHERE true, FOR create, update WHERE false;
DEFINE FIELD OVERWRITE delay ON job TYPE int DEFAULT ALWAYS 0 PERMISSIONS FOR select, create WHERE true, FOR update WHERE false;
