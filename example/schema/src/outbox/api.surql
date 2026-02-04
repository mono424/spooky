-- ##################################################################
-- API OUTBOX TABLE
-- ##################################################################

DEFINE TABLE job SCHEMAFULL
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD assigned_to ON TABLE job TYPE record<thread>
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD path ON TABLE job TYPE string
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD payload ON TABLE job TYPE any
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD retries ON TABLE job TYPE int DEFAULT ALWAYS 0
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD max_retries ON TABLE job TYPE int DEFAULT ALWAYS 3;

DEFINE FIELD retry_strategy ON TABLE job TYPE string DEFAULT ALWAYS "linear"
ASSERT $value IN ["linear", "exponential"]
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD status ON TABLE job TYPE string DEFAULT ALWAYS "pending"
ASSERT $value IN ["pending", "processing", "success", "failed"]
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD errors ON TABLE job TYPE array<object> DEFAULT ALWAYS []
PERMISSIONS
  FOR create WHERE true
  FOR select, update WHERE false;

DEFINE FIELD updated_at ON TABLE job TYPE datetime
DEFAULT ALWAYS time::now()
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD created_at ON TABLE job TYPE datetime
VALUE time::now()
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;
