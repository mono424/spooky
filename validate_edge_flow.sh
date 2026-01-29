#!/bin/bash
set -e

# Configuration
SSP_URL=${SSP_URL:-"http://localhost:8667"}
SURREAL_URL=${SURREAL_URL:-"http://localhost:8666"}
SECRET=${SECRET:-"spooky-admin-secret"} 
NS="test"
DB="test"

echo "=== SSP Edge Synchronization Validation ==="
echo "SSP URL: $SSP_URL"
echo "SurrealDB URL: $SURREAL_URL"

# Helper to check if server is up
check_server() {
    if ! curl -s -H "Authorization: Bearer $SECRET" "$SSP_URL/health" > /dev/null; then
        echo "❌ Error: SSP server is not reachable at $SSP_URL"
        echo "Please start the server in a separate terminal: cargo run --bin ssp-server"
        exit 1
    fi
}

check_server

echo ""
echo "--- Step 1: Resetting State ---"
HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$SSP_URL/reset" \
  -H "Authorization: Bearer $SECRET")

if [ "$HTTP_STATUS" -eq 200 ]; then
    echo "✅ State reset (HTTP 200)"
else
    echo "❌ Reset failed (HTTP $HTTP_STATUS)"
    exit 1
fi

echo ""
echo "--- Step 1.5: Creating _spooky_version Record (Simulating external system) ---"
VERSION_RES=$(curl -s -X POST "$SURREAL_URL/sql" \
    -u "root:root" \
    -H "NS: $NS" \
    -H "DB: $DB" \
    -H "Accept: application/json" \
    -d "USE NS $NS DB $DB; CREATE _spooky_version SET record_id = users:1, version = 1")

echo "Version Creation Response: $VERSION_RES"

echo ""
echo "--- Step 2: Ingesting Record (users:1) ---"
INGEST_RES=$(curl -s -X POST "$SSP_URL/ingest" \
  -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{"table":"users","op":"CREATE","id":"users:1","record":{"name":"Alice","age":30}}')

echo "Ingest Response: $INGEST_RES"

echo ""
echo "--- Step 3: Registering View (Scenario A) ---"
VIEW_DEF='{
  "id": "incantation:active_users",
  "clientId": "client_1",
  "surql": "SELECT * FROM users WHERE age > 20",
  "ttl": "1h",
  "lastActiveAt": "2023-01-01T00:00:00Z",
  "safe_params": {}
}'

REGISTER_RES=$(curl -s -X POST "$SSP_URL/view/register" \
  -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d "$VIEW_DEF")

echo "Register Response: $REGISTER_RES"

echo ""
echo "--- Step 4: Verifying Edge Creation ---"
# We need to query SurrealDB to check if the edge exists.
# We'll use curl to SurrealDB /sql endpoint
QUERY="SELECT * FROM _spooky_list_ref WHERE in = 'incantation:active_users'"
EDGE_CHECK=$(curl -s -X POST "$SURREAL_URL/sql" \
    -u "root:root" \
    -H "NS: $NS" \
    -H "DB: $DB" \
    -H "Accept: application/json" \
    -d "USE NS $NS DB $DB; $QUERY")

echo "SurrealDB Response: $EDGE_CHECK"

if echo "$EDGE_CHECK" | grep -q "users:1"; then
    echo "✅ SUCCESS: Edge found for users:1"
else
    echo "❌ FAILURE: Edge NOT found for users:1"
    # Don't exit, keep checking re-reg
fi

echo ""
echo "--- Step 5: Re-registering View (Scenario B - Cleanup) ---"
# We register the same view again. This should trigger cleanup logs in the server
# and ensuring edges are re-created (or at least consistent).

REGISTER_RES_2=$(curl -s -X POST "$SSP_URL/view/register" \
  -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d "$VIEW_DEF")

echo "Re-register Response: $REGISTER_RES_2"

echo ""
echo "--- Step 6: Verifying Edge Persistence ---"
EDGE_CHECK_2=$(curl -s -X POST "$SURREAL_URL/sql" \
    -u "root:root" \
    -H "NS: $NS" \
    -H "DB: $DB" \
    -H "Accept: application/json" \
    -d "USE NS $NS DB $DB; $QUERY")

echo "SurrealDB Response: $EDGE_CHECK_2"

if echo "$EDGE_CHECK_2" | grep -q "users:1"; then
    echo "✅ SUCCESS: Edge persisted after re-registration"
else
    echo "❌ FAILURE: Edge lost after re-registration"
fi

echo "=== Validation Complete ==="
