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
check_table() {
  curl -s -X POST "${SURREAL_URL}/sql" \
    -H 'Accept: application/json' \
    -H "surreal-ns: ${NS}" \
    -H "surreal-db: ${DB}" \
    -u "${USER}:${PASS}" \
    --data "INFO FOR DB;"
}

# Wait for DB to be ready
until check_table > /dev/null; do
  echo "Waiting for SurrealDB..."
  sleep 2
done

# Check if _spooky_incantation exists
RESPONSE=$(check_table)

if echo "$RESPONSE" | grep -q "_spooky_incantation"; then
  echo "Table '_spooky_incantation' found. Skipping migration."
else
  echo "Table '_spooky_incantation' NOT found. Running migration..."
  curl -s -X POST "${SURREAL_URL}/sql" \
    -H 'Accept: application/json' \
    -H "surreal-ns: ${NS}" \
    -H "surreal-db: ${DB}" \
    -u "${USER}:${PASS}" \
    --data-binary "@${SCHEMA_FILE}"
    
  echo "Migration completed."
fi
