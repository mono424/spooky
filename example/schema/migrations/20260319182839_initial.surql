-- Migration: initial
-- Created: 2026-03-19T18:28:40.822400+00:00

-- Removed
REMOVE FUNCTION IF EXISTS fn::query::heartbeat;
REMOVE FUNCTION IF EXISTS fn::query::register;
REMOVE API IF EXISTS '/spooky';
REMOVE EVENT IF EXISTS _00_comment_delete ON TABLE comment;
REMOVE EVENT IF EXISTS _00_comment_mutation ON TABLE comment;
REMOVE EVENT IF EXISTS _00_job_delete ON TABLE job;
REMOVE EVENT IF EXISTS _00_job_mutation ON TABLE job;
REMOVE EVENT IF EXISTS _00_thread_delete ON TABLE thread;
REMOVE EVENT IF EXISTS _00_thread_mutation ON TABLE thread;
REMOVE EVENT IF EXISTS _00_user_delete ON TABLE user;
REMOVE EVENT IF EXISTS _00_user_mutation ON TABLE user;

-- Modified
DEFINE EVENT OVERWRITE _00_dbsp_cleanup ON _00_query WHEN $event = 'DELETE' THEN { LET $payload = { id: <string> $before.id }; LET $result = http::post('http://localhost:9667/view/unregister', $payload, { Authorization: 'Bearer ' }); };
