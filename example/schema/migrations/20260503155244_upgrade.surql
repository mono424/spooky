-- Migration: upgrade
-- Created: 2026-05-03T15:52:45.740579+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_cursor TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE fn::can_update_record(record_id), FOR delete NONE;
