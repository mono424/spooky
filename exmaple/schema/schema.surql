-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################
DEFINE SCOPE user_auth
    SESSION 14d
    SIGNUP (
        CREATE user SET
            id = $token.sub,
            email = $token.email,
            username = $token.username,
            created_at = time::now()
    )
    SIGNIN (
        SELECT * FROM user WHERE id = $token.sub
    )
;

-- ##################################################################
-- USER TABLE
-- ##################################################################

DEFINE TABLE user SCHEMAFULL
    PERMISSIONS
        FOR select, update, delete WHERE id = $auth.id
;

DEFINE FIELD username ON user TYPE string
    ASSERT $value != NONE AND string::is::alphanum($value) AND string::len($value) > 3;
DEFINE INDEX unique_username ON user FIELDS username UNIQUE;

DEFINE FIELD email ON user TYPE string
    ASSERT string::is::email($value);
DEFINE INDEX unique_email ON user FIELDS email UNIQUE;

DEFINE FIELD created_at ON user TYPE datetime
    VALUE $value OR time::now();

DEFINE FIELD friends ON user TYPE array<record<user>>;


-- ##################################################################
-- FRIEND REQUEST TABLE
-- ##################################################################

DEFINE TABLE friend_request SCHEMAFULL
    PERMISSIONS
        FOR create WHERE from = $auth.id
        FOR select WHERE from = $auth.id OR to = $auth.id
        FOR update WHERE to = $auth.id
        FOR delete WHERE from = $auth.id AND status = 'pending'
;

DEFINE FIELD from ON friend_request TYPE record<user>
    VALUE $auth.id;

DEFINE FIELD to ON friend_request TYPE record<user>;

DEFINE FIELD status ON friend_request TYPE string
    ASSERT $value IN ['pending', 'accepted', 'rejected']
    VALUE $value OR 'pending';

DEFINE FIELD created_at ON friend_request TYPE datetime
    VALUE $value OR time::now();


-- ##################################################################
-- MESSAGE TABLE
-- ##################################################################

DEFINE TABLE message SCHEMAFULL
    PERMISSIONS
        FOR create WHERE from = $auth.id AND (SELECT VALUE id FROM ONLY $auth.friends) CONTAINS to
        FOR select WHERE from = $auth.id OR to = $auth.id
        FOR delete WHERE from = $auth.id
;

DEFINE FIELD from ON message TYPE record<user>
    VALUE $auth.id;

DEFINE FIELD to ON message TYPE record<user>;

DEFINE FIELD content ON message TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD sent_at ON message TYPE datetime
    VALUE $value OR time::now();

DEFINE FIELD read_at ON message TYPE option<datetime>;

