-- ##################################################################
-- API OUTBOX TABLE
-- ##################################################################

DEFINE TABLE backend_api_outbox SCHEMAFULL
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD assigned_to ON TABLE backend_api_outbox TYPE record<thread>
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD path ON TABLE backend_api_outbox TYPE string
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD payload ON TABLE backend_api_outbox TYPE object
ASSERT $value.id = <string> assigned_to.id
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD retries ON TABLE backend_api_outbox TYPE int DEFAULT 0
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD max_retries ON TABLE backend_api_outbox TYPE int DEFAULT 3
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD retry_strategy ON TABLE backend_api_outbox TYPE string DEFAULT "linear"
ASSERT $value IN ["linear", "exponential"]
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD status ON TABLE backend_api_outbox TYPE string DEFAULT "pending"
ASSERT $value IN ["pending", "success", "failed"]
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id;

DEFINE FIELD updated_at ON TABLE backend_api_outbox TYPE datetime
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id
VALUE time::now();

DEFINE FIELD created_at ON TABLE backend_api_outbox TYPE datetime
PERMISSIONS
  FOR select, create, update, delete WHERE $access = "account" AND assigned_to.author.id = $auth.id
VALUE time::now();
