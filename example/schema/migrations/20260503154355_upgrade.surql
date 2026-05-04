-- Migration: upgrade
-- Created: 2026-05-03T15:43:56.633762+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_crdt TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update WHERE fn::can_update_record(record_id), FOR delete NONE;
DEFINE TABLE OVERWRITE _00_list_ref TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE clientId = $session.clientId, FOR create, update, delete NONE;
DEFINE TABLE OVERWRITE _00_query TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE clientId = $session.clientId, FOR delete NONE;
DEFINE TABLE OVERWRITE _00_version TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE record_id.id != NONE, FOR create, update, delete NONE;
DEFINE FIELD OVERWRITE clientId ON _00_list_ref TYPE string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE in ON _00_list_ref TYPE record PERMISSIONS FULL;
DEFINE FIELD OVERWRITE out ON _00_list_ref TYPE record PERMISSIONS FULL;
DEFINE FIELD OVERWRITE parent ON _00_list_ref TYPE none | record<_00_list_ref> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE parent_rel ON _00_list_ref TYPE none | string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE updatedAt ON _00_list_ref TYPE datetime READONLY VALUE time::now() PERMISSIONS FULL;
DEFINE FIELD OVERWRITE version ON _00_list_ref TYPE int PERMISSIONS FULL;
DEFINE FIELD OVERWRITE clientId ON _00_query TYPE none | string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE lastActiveAt ON _00_query TYPE datetime DEFAULT time::now() PERMISSIONS FULL;
DEFINE FIELD OVERWRITE surql ON _00_query TYPE none | string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE ttl ON _00_query TYPE duration PERMISSIONS FULL;

-- Added
DEFINE FUNCTION OVERWRITE fn::can_update_record($r: record) -> bool { RETURN array::len((UPDATE $r SET _00_rv = _00_rv RETURN BEFORE)) > 0 } PERMISSIONS FULL;
DEFINE TABLE OVERWRITE _00_cursor TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE fn::can_update_record(record_id), FOR delete NONE;
DEFINE FIELD OVERWRITE field ON _00_cursor TYPE string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE record_id ON _00_cursor TYPE record PERMISSIONS FULL;
DEFINE FIELD OVERWRITE state ON _00_cursor TYPE none | string PERMISSIONS FULL;
DEFINE INDEX OVERWRITE idx_cursor_record_field ON _00_cursor FIELDS record_id, field UNIQUE;
