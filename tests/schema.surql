-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################
DEFINE ACCESS account ON DATABASE TYPE RECORD
	SIGNUP {
		IF string::len($username) <= 3 { THROW "Username must be longer than 3 characters" };
		IF string::len($password) == 0 { THROW "Password cannot be empty" };

		LET $existing = (SELECT value id FROM user WHERE username = $username LIMIT 1)[0];
		IF $existing != NONE { THROW "Username '" + <string>$username + "' is already taken" };

		LET $u = CREATE user SET username = $username, password = crypto::argon2::generate($password);
		RETURN $u;
	}
	SIGNIN ( SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password) )
	DURATION FOR TOKEN 365d, FOR SESSION 365d
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

-- ##################################################################
-- THREAD TABLE
-- ##################################################################

DEFINE TABLE thread SCHEMAFULL
  PERMISSIONS
    FOR select WHERE true
    FOR update, delete, create WHERE $access = "account" AND author.id = $auth.id
;


DEFINE FIELD title ON TABLE thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0 AND string::len($value) <= 200;

DEFINE FIELD content ON TABLE thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD author ON TABLE thread TYPE record<user>;

DEFINE FIELD created_at ON TABLE thread TYPE datetime
    VALUE time::now();

-- ##################################################################
-- COMMENT TABLE
-- ##################################################################

DEFINE TABLE comment SCHEMAFULL
  PERMISSIONS
    FOR select WHERE true
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
  PERMISSIONS FOR select WHERE true;

DEFINE EVENT comment_created ON TABLE comment WHEN $event = "CREATE" THEN
  RELATE ($after.id)->commented_on->($after.thread)
;