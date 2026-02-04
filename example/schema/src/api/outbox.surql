-- ##################################################################
-- API OUTBOX TABLE
-- ##################################################################

DEFINE TABLE backend_api_outbox SCHEMAFULL
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD assigned_to ON TABLE backend_api_outbox TYPE record<thread>;

DEFINE FIELD path ON TABLE backend_api_outbox TYPE string;

DEFINE FIELD payload ON TABLE backend_api_outbox TYPE any;

DEFINE FIELD retries ON TABLE backend_api_outbox TYPE int DEFAULT 0;

DEFINE FIELD max_retries ON TABLE backend_api_outbox TYPE int DEFAULT 3;

DEFINE FIELD retry_strategy ON TABLE backend_api_outbox TYPE string DEFAULT "linear"
ASSERT $value IN ["linear", "exponential"];

DEFINE FIELD status ON TABLE backend_api_outbox TYPE string DEFAULT "pending"
ASSERT $value IN ["pending", "processing", "success", "failed"];

DEFINE FIELD updated_at ON TABLE backend_api_outbox TYPE datetime
VALUE time::now();

DEFINE FIELD created_at ON TABLE backend_api_outbox TYPE datetime
VALUE time::now();
