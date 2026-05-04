-- Migration: security
-- Created: 2026-05-04T17:05:02.921363+00:00

-- Modified
DEFINE ACCESS OVERWRITE account ON DATABASE TYPE RECORD SIGNUP { LET $u = (CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password), share_pubkey = $share_pubkey); IF $share_privkey != NONE THEN CREATE user_keypair SET owner = $u.id, privkey = $share_privkey END; RETURN $u; } SIGNIN (SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password)) WITH JWT ALGORITHM HS512 KEY '[REDACTED]' WITH ISSUER KEY '[REDACTED]' DURATION FOR TOKEN 15m, FOR SESSION 4w2d;
