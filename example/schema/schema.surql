-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################
DEFINE SCOPE user_auth
    SESSION 14d
    SIGNUP (
        CREATE user SET
            id = $token.sub,
            username = $token.username,
            password = crypto::argon2::generate($token.password),
            created_at = time::now()
    )
    SIGNIN (
        SELECT * FROM user WHERE username = $token.username AND crypto::argon2::compare(password, $token.password)
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

DEFINE FIELD password ON user TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD created_at ON user TYPE datetime
    VALUE $value OR time::now();

-- ##################################################################
-- THREAD TABLE
-- ##################################################################

DEFINE TABLE thread SCHEMAFULL
    PERMISSIONS
        FOR create WHERE author = $auth.id
        FOR select WHERE true
        FOR update WHERE author = $auth.id
        FOR delete WHERE author = $auth.id
;

DEFINE FIELD title ON thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0 AND string::len($value) <= 200;

DEFINE FIELD content ON thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD author ON thread TYPE record<user>
    VALUE $auth.id;

DEFINE FIELD created_at ON thread TYPE datetime
    VALUE $value OR time::now();


-- ##################################################################
-- COMMENT TABLE
-- ##################################################################

DEFINE TABLE comment SCHEMAFULL
    PERMISSIONS
        FOR create WHERE author = $auth.id
        FOR select WHERE true
        FOR update WHERE author = $auth.id
        FOR delete WHERE author = $auth.id
;

DEFINE FIELD thread_id ON comment TYPE record<thread>;

DEFINE FIELD content ON comment TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD author ON comment TYPE record<user>
    VALUE $auth.id;

DEFINE FIELD created_at ON comment TYPE datetime
    VALUE $value OR time::now();

