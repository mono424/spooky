import 'dart:convert';
import 'package:crypto/crypto.dart' as crypto;

String computeSha1(String input) {
  var bytes = utf8.encode(input);
  var digest = crypto.sha1.convert(bytes);
  return digest.toString();
}

void main() {
  final input =
      r"""-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################
DEFINE ACCESS account ON DATABASE TYPE RECORD
	SIGNUP ( CREATE user SET username = $username, password = crypto::argon2::generate($password) )
	SIGNIN ( SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password) )
	DURATION FOR TOKEN 15m, FOR SESSION 12h
;

-- ##################################################################
-- USER TABLE
-- ##################################################################

DEFINE TABLE user SCHEMAFULL
  PERMISSIONS FOR select, update, delete, create
  WHERE $access = "account"
  AND id = $auth.id
;

DEFINE FIELD username ON TABLE user TYPE string
    ASSERT $value != NONE AND string::is::alphanum($value) AND string::len($value) > 3;
    
DEFINE INDEX unique_username ON TABLE user FIELDS username UNIQUE;

DEFINE FIELD password ON TABLE user TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD created_at ON TABLE user TYPE datetime
    VALUE time::now();

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

DEFINE FIELD thread_id ON TABLE comment TYPE record<thread>;

DEFINE FIELD content ON TABLE comment TYPE string
    ASSERT $value != NONE AND string::len($value) > 0;

DEFINE FIELD author ON TABLE comment TYPE record<user>;

DEFINE FIELD created_at ON TABLE comment TYPE datetime
    VALUE time::now();

""";
  final hash = computeSha1(input);
  print("Input: '$input'");
  print("SHA1: $hash");
}
