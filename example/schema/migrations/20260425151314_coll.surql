-- Migration: coll
-- Created: 2026-04-25T15:13:17.401723+00:00

-- Modified
DEFINE TABLE OVERWRITE thread TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE true, FOR create, delete WHERE $access = 'account' AND author.id = $auth.id, FOR update WHERE $access = 'account' AND (author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id));

-- Added
DEFINE TABLE OVERWRITE collaborates_on TYPE RELATION IN user OUT thread SCHEMAFULL PERMISSIONS FOR select WHERE true, FOR create WHERE $access = 'account' AND in = $auth.id AND $parent.out INSIDE (SELECT VALUE thread FROM thread_invite), FOR update WHERE false, FOR delete WHERE $access = 'account' AND (in.id = $auth.id OR out.author.id = $auth.id);
DEFINE TABLE OVERWRITE thread_invite TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE true, FOR create, delete WHERE $access = 'account' AND thread.author.id = $auth.id, FOR update WHERE false;
DEFINE FIELD OVERWRITE in ON collaborates_on TYPE record<user> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE out ON collaborates_on TYPE record<thread> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE created_at ON thread_invite TYPE datetime VALUE time::now() PERMISSIONS FULL;
DEFINE FIELD OVERWRITE created_by ON thread_invite TYPE record<user> VALUE $auth.id PERMISSIONS FULL;
DEFINE FIELD OVERWRITE thread ON thread_invite TYPE record<thread> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE token ON thread_invite TYPE string ASSERT $value != NONE AND string::len($value) >= 16 PERMISSIONS FULL;
DEFINE INDEX OVERWRITE unique_collab ON collaborates_on FIELDS in, out UNIQUE;
DEFINE INDEX OVERWRITE unique_invite_token ON thread_invite FIELDS token UNIQUE;
