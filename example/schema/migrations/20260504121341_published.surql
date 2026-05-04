-- Migration: published
-- Created: 2026-05-04T12:13:43.459015+00:00

-- Modified
DEFINE TABLE OVERWRITE comment TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE thread.published = true OR $access = 'account' AND (thread.author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.thread)), FOR create, update, delete WHERE $access = 'account' AND author.id = $auth.id;
DEFINE TABLE OVERWRITE commented_on TYPE RELATION IN comment OUT thread SCHEMAFULL PERMISSIONS FOR select WHERE out.published = true OR $access = 'account' AND (out.author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.out)), FOR create, update, delete NONE;
DEFINE TABLE OVERWRITE thread TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE published = true OR $access = 'account' AND (author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id)), FOR create, delete WHERE $access = 'account' AND author.id = $auth.id, FOR update WHERE $access = 'account' AND (author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id));

-- Added
DEFINE FIELD OVERWRITE published ON thread TYPE bool VALUE $value OR false PERMISSIONS FULL;
