-- Migration: upgrade
-- Created: 2026-05-03T16:34:25.054382+00:00

-- Modified
DEFINE TABLE OVERWRITE _00_list_ref TYPE ANY SCHEMALESS PERMISSIONS FOR select WHERE clientId = <string> session::id(), FOR create, update, delete NONE;
DEFINE TABLE OVERWRITE _00_query TYPE ANY SCHEMALESS PERMISSIONS FOR select, create, update WHERE clientId = <string> session::id(), FOR delete NONE;
