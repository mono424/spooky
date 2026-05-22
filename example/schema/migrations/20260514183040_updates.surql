-- Migration: updates
-- Created: 2026-05-14T18:30:41.931614+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_list_ref TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE auth_id = <string> $auth.id, FOR create, update, delete NONE;

-- Added
DEFINE FIELD OVERWRITE auth_id ON _00_list_ref TYPE string DEFAULT '' PERMISSIONS FULL;
DEFINE FIELD OVERWRITE auth_id ON _00_query TYPE string DEFAULT '' PERMISSIONS FULL;
