-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################

DEFINE ACCESS account ON DATABASE TYPE RECORD
  SIGNUP {
    -- The private key lives in `user_keypair` so its table-level SELECT
    -- rule (`owner = $auth`) gates it consistently — field-level rules in
    -- SurrealDB v3 only enforce on record-id queries, not table scans.
    LET $u = CREATE ONLY user SET
      username     = $username,
      password     = crypto::argon2::generate($password),
      share_pubkey = $share_pubkey;
    CREATE user_keypair SET owner = $u.id, privkey = $share_privkey;
    RETURN $u;
  }
  SIGNIN ( SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password) )
  DURATION FOR TOKEN 15m, FOR SESSION 30d
;

-- ##################################################################
-- USER TABLE
-- ##################################################################

DEFINE TABLE user SCHEMAFULL
PERMISSIONS
  FOR update, delete WHERE $access = "account" AND id = $auth.id
  FOR create, select WHERE true;

DEFINE FIELD username ON TABLE user TYPE string
ASSERT $value != NONE AND string::len($value) > 3
PERMISSIONS
    FOR select WHERE true
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;
    
DEFINE INDEX unique_username ON TABLE user FIELDS username UNIQUE;

DEFINE FIELD password ON TABLE user TYPE string
ASSERT $value != NONE AND string::len($value) > 0
PERMISSIONS
    FOR select WHERE false
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

DEFINE FIELD created_at ON TABLE user TYPE datetime
VALUE time::now()
PERMISSIONS
    FOR select WHERE false
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

DEFINE FIELD profile_picture ON TABLE user TYPE option<string>
PERMISSIONS
    FOR select WHERE true
    FOR create, update WHERE $access = "account" AND id = $auth.id;

-- Ed25519 public key (PEM SPKI). Read by anyone — used to verify share-link
-- JWTs the user has signed.
DEFINE FIELD share_pubkey ON TABLE user TYPE option<string>
PERMISSIONS
    FOR select WHERE true
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

-- The matching private key lives in the `user_keypair` table so its
-- table-level SELECT rule scopes it to the owner. Field-level rules
-- aren't enough: SurrealDB v3 only enforces field SELECT permissions on
-- record-id queries, so a free-form WHERE scan would leak the key
-- otherwise.
DEFINE TABLE user_keypair SCHEMAFULL
PERMISSIONS
    FOR select, update WHERE owner = $auth
    FOR create WHERE true
    FOR delete WHERE false;
DEFINE FIELD owner    ON TABLE user_keypair TYPE record<user>;
DEFINE FIELD privkey  ON TABLE user_keypair TYPE string;
DEFINE INDEX unique_keypair_owner ON TABLE user_keypair FIELDS owner UNIQUE;

-- ##################################################################
-- THREAD TABLE
-- ##################################################################

DEFINE TABLE thread SCHEMAFULL
  PERMISSIONS
    FOR select WHERE published = true OR ($access = "account" AND (author.id = $auth.id OR $auth.id IN (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id)))
    FOR create WHERE $access = "account" AND author.id = $auth.id
    FOR update WHERE $access = "account"
      AND (author.id = $auth.id
           OR $auth.id IN (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id))
    FOR delete WHERE $access = "account" AND author.id = $auth.id
;

-- @crdt text
DEFINE FIELD title ON TABLE thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0 AND string::len($value) <= 200;

-- @crdt text
DEFINE FIELD content ON TABLE thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD title_suggestion ON TABLE thread TYPE option<string>;

DEFINE FIELD content_suggestion ON TABLE thread TYPE option<string>;

DEFINE FIELD author ON TABLE thread TYPE record<user>; -- @parent

DEFINE FIELD created_at ON TABLE thread TYPE datetime
    VALUE time::now();

DEFINE FIELD active ON TABLE thread TYPE bool VALUE $value OR false;

DEFINE FIELD published ON TABLE thread TYPE bool VALUE $value OR false;

-- ##################################################################
-- COMMENT TABLE
-- ##################################################################

DEFINE TABLE comment SCHEMAFULL
  PERMISSIONS
    FOR select WHERE thread.published = true OR ($access = "account" AND (thread.author.id = $auth.id OR $auth.id IN (SELECT VALUE in FROM collaborates_on WHERE out = $parent.thread)))
    FOR update, delete, create WHERE $access = "account" AND author.id = $auth.id
;

DEFINE FIELD thread ON TABLE comment TYPE record<thread>; -- @parent

DEFINE FIELD content ON TABLE comment TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD author ON TABLE comment TYPE record<user>;

DEFINE FIELD created_at ON TABLE comment TYPE datetime
    VALUE time::now();

-- ##################################################################
-- RELATION TABLES
-- ##################################################################

DEFINE TABLE commented_on SCHEMAFULL TYPE RELATION
  FROM comment TO thread
  PERMISSIONS
    FOR select WHERE out.published = true OR ($access = "account" AND (out.author.id = $auth.id OR $auth.id IN (SELECT VALUE in FROM collaborates_on WHERE out = $parent.out)));

DEFINE EVENT comment_created ON TABLE comment WHEN $event = "CREATE" THEN
  RELATE ($after.id)->commented_on->($after.thread)
;

-- ##################################################################
-- COLLABORATORS
-- ##################################################################

-- Per-issuer history of share links. Append-only by design — there's no
-- way to revoke a JWT once it's out, so the dialog only shows the issuer
-- the links they've made and warns them. Acceptance happens server-side
-- (example/api /share/accept), not via this table; rows here are purely
-- informational.
DEFINE TABLE share_link SCHEMAFULL
  PERMISSIONS
    FOR select WHERE issuer.id = $auth.id
    FOR create WHERE $access = "account" AND issuer = $auth.id AND thread.author.id = $auth.id
    FOR update, delete WHERE false;

DEFINE FIELD issuer     ON TABLE share_link TYPE record<user> VALUE $auth.id;
DEFINE FIELD thread     ON TABLE share_link TYPE record<thread>;
DEFINE FIELD jwt        ON TABLE share_link TYPE string;
DEFINE FIELD jti        ON TABLE share_link TYPE string;
DEFINE FIELD exp        ON TABLE share_link TYPE datetime;
DEFINE FIELD created_at ON TABLE share_link TYPE datetime VALUE time::now();
DEFINE INDEX unique_share_jti ON TABLE share_link FIELDS jti UNIQUE;

-- Only root may CREATE a `collaborates_on` edge — the JWT-verifying API
-- endpoint does it on the recipient's behalf after checking the share
-- link's signature against the issuer's `share_pubkey`. Author can still
-- kick collaborators off via FOR delete.
DEFINE TABLE collaborates_on SCHEMAFULL TYPE RELATION FROM user TO thread
  PERMISSIONS
    FOR select WHERE true
    FOR create WHERE false
    FOR delete WHERE $access = "account"
      AND (in.id = $auth.id OR out.author.id = $auth.id)
    FOR update WHERE false;

DEFINE INDEX unique_collab ON TABLE collaborates_on FIELDS in, out UNIQUE;