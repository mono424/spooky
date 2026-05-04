-- Migration: upgrade
-- Created: 2026-05-03T18:15:18.567931+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_crdt TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE fn::can_update_record(record_id), FOR delete NONE;
DEFINE TABLE OVERWRITE _00_cursor TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE fn::can_update_record(record_id), FOR delete NONE;
