-- Migration: share_jwt
-- Created: 2026-05-04T12:29:48.378284+00:00

-- Removed
REMOVE TABLE IF EXISTS thread_invite;
REMOVE FIELD IF EXISTS created_at ON TABLE thread_invite;
REMOVE FIELD IF EXISTS created_by ON TABLE thread_invite;
REMOVE FIELD IF EXISTS thread ON TABLE thread_invite;
REMOVE FIELD IF EXISTS token ON TABLE thread_invite;
REMOVE INDEX IF EXISTS unique_invite_token ON TABLE thread_invite;

-- Modified
DEFINE ACCESS OVERWRITE account ON DATABASE TYPE RECORD SIGNUP (CREATE user SET username = $username, password = crypto::argon2::generate($password), share_pubkey = $share_pubkey, share_privkey = $share_privkey) SIGNIN (SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password)) WITH JWT ALGORITHM HS512 KEY '[REDACTED]' WITH ISSUER KEY '[REDACTED]' DURATION FOR TOKEN 15m, FOR SESSION 4w2d;
DEFINE TABLE OVERWRITE collaborates_on TYPE RELATION IN user OUT thread SCHEMAFULL PERMISSIONS FOR select WHERE true, FOR create, update WHERE false, FOR delete WHERE $access = 'account' AND (in.id = $auth.id OR out.author.id = $auth.id);

-- Added
DEFINE TABLE OVERWRITE share_link TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select WHERE issuer.id = $auth.id, FOR create WHERE $access = 'account' AND issuer = $auth.id AND thread.author.id = $auth.id, FOR update, delete WHERE false;
DEFINE FIELD OVERWRITE created_at ON share_link TYPE datetime VALUE time::now() PERMISSIONS FULL;
DEFINE FIELD OVERWRITE exp ON share_link TYPE datetime PERMISSIONS FULL;
DEFINE FIELD OVERWRITE issuer ON share_link TYPE record<user> VALUE $auth.id PERMISSIONS FULL;
DEFINE FIELD OVERWRITE jti ON share_link TYPE string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE jwt ON share_link TYPE string PERMISSIONS FULL;
DEFINE FIELD OVERWRITE thread ON share_link TYPE record<thread> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE share_privkey ON user TYPE none | string PERMISSIONS FOR select WHERE id = $auth.id, FOR create WHERE true, FOR update WHERE $access = 'account' AND id = $auth.id;
DEFINE FIELD OVERWRITE share_pubkey ON user TYPE none | string PERMISSIONS FOR select, create WHERE true, FOR update WHERE $access = 'account' AND id = $auth.id;
DEFINE INDEX OVERWRITE unique_share_jti ON share_link FIELDS jti UNIQUE;
