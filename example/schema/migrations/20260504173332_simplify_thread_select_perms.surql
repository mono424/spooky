-- Migration: simplify_thread_select_perms
-- Created: 2026-05-04T17:33:34.538716+00:00

-- Modified
DEFINE TABLE OVERWRITE comment TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE thread.published = true OR $access = 'account' AND thread.author.id = $auth.id, FOR create, update, delete WHERE $access = 'account' AND author.id = $auth.id;
DEFINE TABLE OVERWRITE commented_on TYPE RELATION IN comment OUT thread SCHEMAFULL PERMISSIONS FOR select WHERE out.published = true OR $access = 'account' AND out.author.id = $auth.id, FOR create, update, delete NONE;
DEFINE TABLE OVERWRITE thread TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE published = true OR $access = 'account' AND author.id = $auth.id, FOR create, delete WHERE $access = 'account' AND author.id = $auth.id, FOR update WHERE $access = 'account' AND (author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id));
