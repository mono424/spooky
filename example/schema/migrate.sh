#!/bin/sh

# Configuration
SURREAL_URL="http://surrealdb:8000"
NS="main"
DB="main"
USER="root"
PASS="root"
SCHEMA_FILE="/remote.gen.surql"

echo "Checking if database is initialized..."

# Function to check table existence
# Function to check table existence
check_table() {
  curl -sS -X POST "${SURREAL_URL}/sql" \
    -H 'Accept: application/json' \
    -H "surreal-ns: ${NS}" \
    -H "surreal-db: ${DB}" \
    -u "${USER}:${PASS}" \
    --data "INFO FOR DB;"
}

# Wait for DB to be ready
retries=0
max_retries=30
while [ $retries -lt $max_retries ]; do
  if check_table > /dev/null 2>&1; then
    echo "SurrealDB is ready."
    break
  fi
  echo "Waiting for SurrealDB... ($retries/$max_retries)"
  sleep 2
  retries=$((retries+1))
done

if [ $retries -eq $max_retries ]; then
  echo "Error: Timed out waiting for SurrealDB."
  exit 1
fi

# Check if _spooky_incantation exists
RESPONSE=$(check_table)

# Check for curl error or invalid response
if [ -z "$RESPONSE" ]; then
  echo "Error: Empty response from SurrealDB."
  exit 1
fi

if echo "$RESPONSE" | grep -q "_spooky_incantation"; then
  echo "Table '_spooky_incantation' found. Skipping migration."
else
  echo "Table '_spooky_incantation' NOT found. Running migration..."
  # Run migration and capture output
  MIGRATION_OUTPUT=$(curl -sS -X POST "${SURREAL_URL}/sql" \
    -H 'Accept: application/json' \
    -H "surreal-ns: ${NS}" \
    -H "surreal-db: ${DB}" \
    -u "${USER}:${PASS}" \
    --data-binary "@${SCHEMA_FILE}")
  
  echo "Migration Output: $MIGRATION_OUTPUT"
  
  # Check for errors in output
  if echo "$MIGRATION_OUTPUT" | grep -q '"code":400'; then
      echo "Error during migration!"
      exit 1
  fi
    
  echo "Migration completed."
fi
