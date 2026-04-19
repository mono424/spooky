-- Migration: test
-- Created: 2026-04-15T13:35:11.031217+00:00

-- Modified
DEFINE EVENT OVERWRITE _00_dbsp_cleanup ON _00_query WHEN $event = 'DELETE' THEN { LET $payload = { id: <string> $before.id }; LET $result = http::post($sp00ky_endpoint + '/view/unregister', $payload, { Authorization: 'Bearer ' + $sp00ky_secret }); };
