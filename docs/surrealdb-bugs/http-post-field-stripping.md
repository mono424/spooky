# `http::post` silently drops fields from the body object

**Observed on:** SurrealDB `v3.0.5`, still present on `v3.1.0-beta.3` (unverified).

## Symptom

When a SurrealQL function builds an object literal and passes it as the body to `http::post(url, body, headers)`, any field added at the top level of `body` after a certain set is silently stripped before the HTTP request goes out. The receiver sees only the original keys.

There is no error in the SurrealQL response. The function appears to run cleanly.

## Reproduction

1. Start an HTTP capture sink: `nc -l 4444` (or any echo server).
2. Run via the SurrealDB CLI:

   ```surql
   LET $body = { existing: 'visible', added: 'invisible' };
   http::post('http://localhost:4444/', $body, {});
   ```

3. Inspect the captured POST body.

**Observed:** `{ existing: 'visible' }` on the wire. The `added` key is gone.

**Expected:** Both fields present.

Nested fields **do** round-trip — anything inside an already-present key survives.

## Workaround

When the SSP needed `auth_id` to reach the receiver via `fn::query::register`, we couldn't add a top-level `authId` to `$module_config`. Instead we route the auth id through `params.auth.id` (which is inside an already-present `params` field, so it survives) and extract it server-side.

**Files changed:**
- `apps/cli/src/functions_remote_*.surql` — embed `auth_id` inside `params`, not as a top-level field.
- `packages/ssp/src/service.rs::prepare_registration_dbsp` — extract `auth_id` from `safe_params_val.auth.id`.
