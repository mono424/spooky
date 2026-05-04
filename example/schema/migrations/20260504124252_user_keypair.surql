-- Migration: user_keypair
-- Created: 2026-05-04T12:42:54.105062+00:00

-- Removed
REMOVE FIELD IF EXISTS share_privkey ON TABLE user;

-- Modified
DEFINE ACCESS OVERWRITE account ON DATABASE TYPE RECORD SIGNUP { LET $u = (CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password), share_pubkey = $share_pubkey); CREATE user_keypair SET owner = $u.id, privkey = $share_privkey; RETURN $u; } SIGNIN (SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password)) WITH JWT ALGORITHM HS512 KEY '[REDACTED]' WITH ISSUER KEY '[REDACTED]' DURATION FOR TOKEN 15m, FOR SESSION 4w2d;

-- Added
DEFINE TABLE OVERWRITE user_keypair TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select, update WHERE owner = $auth, FOR create WHERE true, FOR delete WHERE false;
DEFINE FIELD OVERWRITE owner ON user_keypair TYPE record<user> PERMISSIONS FULL;
DEFINE FIELD OVERWRITE privkey ON user_keypair TYPE string PERMISSIONS FULL;
DEFINE INDEX OVERWRITE unique_keypair_owner ON user_keypair FIELDS owner UNIQUE;
