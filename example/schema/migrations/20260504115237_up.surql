-- Migration: up
-- Created: 2026-05-04T11:52:39.079061+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_crdt TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE record_id.tb() = 'thread' AND $access = 'account' AND (record_id.author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.record_id)), FOR delete NONE;
DEFINE TABLE OVERWRITE _00_cursor TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE record_id.tb() = 'thread' AND $access = 'account' AND (record_id.author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.record_id)), FOR delete NONE;
