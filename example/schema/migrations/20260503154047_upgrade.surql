-- Migration: upgrade
-- Created: 2026-05-03T15:40:48.931253+00:00

-- Removed
REMOVE FUNCTION IF EXISTS fn::can_update_record;
REMOVE TABLE IF EXISTS _00_cursor;
REMOVE FIELD IF EXISTS field ON TABLE _00_cursor;
REMOVE FIELD IF EXISTS record_id ON TABLE _00_cursor;
REMOVE FIELD IF EXISTS state ON TABLE _00_cursor;
REMOVE INDEX IF EXISTS idx_cursor_record_field ON TABLE _00_cursor;

-- Modified
DEFINE TABLE OVERWRITE _00_crdt TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update, delete WHERE true;
DEFINE TABLE OVERWRITE _00_list_ref TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE true, FOR delete NONE;
DEFINE TABLE OVERWRITE _00_query TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE true, FOR delete NONE;
DEFINE TABLE OVERWRITE _00_version TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE true, FOR create, update, delete NONE;
DEFINE FIELD OVERWRITE clientId ON _00_list_ref TYPE string PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE in ON _00_list_ref TYPE record PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE out ON _00_list_ref TYPE record PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE parent ON _00_list_ref TYPE none | record<_00_list_ref> PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE parent_rel ON _00_list_ref TYPE none | string PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE updatedAt ON _00_list_ref TYPE datetime READONLY VALUE time::now() PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE version ON _00_list_ref TYPE int PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE clientId ON _00_query TYPE none | string PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE lastActiveAt ON _00_query TYPE datetime DEFAULT time::now() PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE surql ON _00_query TYPE none | string PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD OVERWRITE ttl ON _00_query TYPE duration PERMISSIONS FOR select, create, update WHERE true;
