-- Migration: up
-- Created: 2026-05-03T20:22:24.643944+00:00

-- Added
DEFINE FIELD OVERWRITE _00_crdt ON thread TYPE none | object FLEXIBLE PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE _00_cursor ON thread TYPE none | object FLEXIBLE PERMISSIONS FOR select, create, update WHERE true;
