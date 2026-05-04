-- Migration: upgrade
-- Created: 2026-05-03T17:30:11.077820+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_crdt TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE true, FOR delete NONE;
DEFINE TABLE OVERWRITE _00_cursor TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE true, FOR delete NONE;
