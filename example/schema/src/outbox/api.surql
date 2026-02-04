-- ##################################################################
-- API OUTBOX TABLE
-- ##################################################################

DEFINE TABLE job SCHEMAFULL
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD assigned_to ON TABLE job TYPE record<thread>;

DEFINE FIELD path ON TABLE job TYPE string;

DEFINE FIELD payload ON TABLE job TYPE any;

DEFINE FIELD retries ON TABLE job TYPE int DEFAULT 0;

DEFINE FIELD max_retries ON TABLE job TYPE int DEFAULT 3;

DEFINE FIELD retry_strategy ON TABLE job TYPE string DEFAULT "linear"
ASSERT $value IN ["linear", "exponential"];

DEFINE FIELD status ON TABLE job TYPE string DEFAULT "pending"
ASSERT $value IN ["pending", "processing", "success", "failed"];

DEFINE FIELD errors ON TABLE job TYPE array<object>;

DEFINE FIELD updated_at ON TABLE job TYPE datetime
VALUE time::now();

DEFINE FIELD created_at ON TABLE job TYPE datetime
VALUE time::now();
