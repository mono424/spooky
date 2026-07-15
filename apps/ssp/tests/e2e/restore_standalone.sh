#!/usr/bin/env bash
# End-to-end test for the STANDALONE SSP backup + restore flow (no scheduler).
#
# Spins up SurrealDB + MinIO in Docker (reusing the scheduler's e2e compose
# file), runs a standalone SSP against them, creates a backup, mutates the DB,
# then restores. A full pass proves:
#   - The standalone SSP exposes the /backup/* maintenance plane behind auth.
#   - Restore wipes + re-imports the main DB over the HTTP engine.
#   - The SSP's circuit is re-bootstrapped from the restored dump
#     (/info table counts match the restored state, /health back to ready).
#
# Usage:  apps/ssp/tests/e2e/restore_standalone.sh
# Requires: docker, docker compose, cargo, curl, jq, python3.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSP_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
REPO_DIR="$(cd "$SSP_DIR/../.." && pwd)"
COMPOSE="docker compose -p sp00ky-e2e-ssp-restore -f $REPO_DIR/apps/scheduler/tests/e2e/docker-compose.yml"

SURREAL_URL="http://localhost:18000"
MINIO_URL="http://localhost:19000"

SSP_PORT=18667
SSP_URL="http://localhost:$SSP_PORT"
AUTH="Authorization: Bearer e2esecret"

SSP_PID=""
SSP_LOG=""

log()  { printf '\033[36m---> %s\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL: %s\033[0m\n' "$*" >&2; exit 1; }

cleanup() {
  set +e
  if [[ -n "$SSP_PID" ]] && kill -0 "$SSP_PID" 2>/dev/null; then
    log "Stopping SSP (pid $SSP_PID)"
    kill "$SSP_PID" 2>/dev/null
    wait "$SSP_PID" 2>/dev/null
  fi
  log "Tearing down containers"
  $COMPOSE down -v >/dev/null 2>&1
  if [[ -n "${SSP_LOG:-}" && -f "$SSP_LOG" && "${KEEP_LOGS:-0}" != "1" ]]; then
    rm -f "$SSP_LOG"
  fi
}
trap cleanup EXIT

wait_http() {
  local url="$1" label="$2" tries="${3:-60}"
  log "Waiting for $label at $url"
  for _ in $(seq 1 "$tries"); do
    if curl -sf -o /dev/null "$url"; then return 0; fi
    sleep 1
  done
  fail "$label never came up at $url"
}

poll_status() {
  local url="$1" kind="$2" tries="${3:-60}"
  for _ in $(seq 1 "$tries"); do
    local body status
    body="$(curl -sf -H "$AUTH" "$url" || true)"
    if [[ -n "$body" ]]; then
      status="$(printf '%s' "$body" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("status",""))' 2>/dev/null || true)"
      printf '  %s status: %s\n' "$kind" "$status" >&2
      case "$status" in
        completed) printf '%s' "$body"; return 0 ;;
        failed)    printf '%s\n' "$body" >&2; fail "$kind failed" ;;
      esac
    fi
    sleep 1
  done
  fail "$kind did not complete within $tries s"
}

sql() {
  curl -sf -u root:root \
    -H 'Accept: application/json' \
    -H 'surreal-ns: sp00ky' -H 'surreal-db: sp00ky' \
    --data "$1" \
    "$SURREAL_URL/sql"
}

thread_count() {
  sql 'SELECT count() FROM thread GROUP ALL;' | python3 -c 'import sys, json
r = json.load(sys.stdin)
try:
    print(r[0]["result"][0]["count"])
except Exception:
    print(0)'
}

circuit_thread_count() {
  curl -sf "$SSP_URL/info" | python3 -c 'import sys, json
r = json.load(sys.stdin)
print(r[0].get("circuit_tables", {}).get("thread", 0))'
}

# ---------------------------------------------------------------------------
log "Starting SurrealDB + MinIO"
$COMPOSE up -d

wait_http "$SURREAL_URL/health"           "SurrealDB"
wait_http "$MINIO_URL/minio/health/live"  "MinIO"

log "Defining namespace/database + seeding a row (SSP bootstraps from it)"
# /sql with ns/db headers does NOT auto-create them (and returns 200 with an
# ERR body when they're missing) — define explicitly and check the result.
curl -sf -u root:root -H 'Accept: application/json' \
  --data "DEFINE NAMESPACE IF NOT EXISTS sp00ky;" \
  "$SURREAL_URL/sql" | grep -q '"status":"OK"' || fail "DEFINE NAMESPACE failed"
curl -sf -u root:root -H 'Accept: application/json' -H 'surreal-ns: sp00ky' \
  --data "DEFINE DATABASE IF NOT EXISTS sp00ky;" \
  "$SURREAL_URL/sql" | grep -q '"status":"OK"' || fail "DEFINE DATABASE failed"
# `_00_query` always exists in production (CLI migrations define it before any
# SSP starts); SurrealDB v3 errors on SELECT from an undefined table.
sql "DEFINE TABLE IF NOT EXISTS _00_query SCHEMALESS;" | grep -q '"status":"OK"' \
  || fail "DEFINE _00_query failed"
sql "CREATE thread SET title = 'hello';" | grep -q '"status":"OK"' || fail "Seed CREATE failed"

log "Building SSP"
( cd "$SSP_DIR" && cargo build -q )

SSP_BIN="$REPO_DIR/target/debug/ssp-server"
[[ -x "$SSP_BIN" ]] || fail "SSP binary not found at $SSP_BIN"

SSP_LOG="$(mktemp)"
log "Starting standalone SSP (logs: $SSP_LOG)"
(
  exec env \
    SPKY_DB_URL="$SURREAL_URL" \
    SPKY_DB_NS=sp00ky SPKY_DB_NAME=sp00ky \
    SPKY_DB_USER=root SPKY_DB_PASS=root \
    SPKY_SSP_LISTEN_ADDR="0.0.0.0:$SSP_PORT" \
    SPKY_AUTH_SECRET=e2esecret \
    S3_ENDPOINT="$MINIO_URL" S3_ACCESS_KEY=minioadmin S3_SECRET_KEY=minioadmin \
    S3_BUCKET=backups S3_REGION=us-east-1 \
    RUST_LOG="${RUST_LOG:-info}" \
    "$SSP_BIN" > "$SSP_LOG" 2>&1
) &
SSP_PID=$!

log "Waiting for SSP to become ready on $SSP_URL"
for _ in $(seq 1 60); do
  if curl -sf -o /dev/null "$SSP_URL/health"; then break; fi
  if ! kill -0 "$SSP_PID" 2>/dev/null; then
    cat "$SSP_LOG"
    fail "SSP exited before becoming ready"
  fi
  sleep 1
done
curl -sf -o /dev/null "$SSP_URL/health" || { cat "$SSP_LOG"; fail "SSP never became ready"; }

[[ "$(circuit_thread_count)" == "1" ]] || fail "Circuit should hold 1 thread row after bootstrap"

BACKUP_ID="e2e-ssp-$(date +%s)"
PROJECT_SLUG="e2e-ssp"

log "Creating backup $BACKUP_ID via standalone SSP"
# Unauthenticated must be rejected — the SSP's backup plane sits behind auth.
UNAUTH_CODE="$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SSP_URL/backup/create" \
  -H 'Content-Type: application/json' \
  -d "{\"backup_id\":\"unauth\",\"project_slug\":\"x\"}")"
[[ "$UNAUTH_CODE" == "401" || "$UNAUTH_CODE" == "403" ]] \
  || fail "Unauthenticated /backup/create should be rejected, got $UNAUTH_CODE"

curl -sf -X POST "$SSP_URL/backup/create" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"backup_id\":\"$BACKUP_ID\",\"project_slug\":\"$PROJECT_SLUG\"}" >/dev/null

BACKUP_STATE="$(poll_status "$SSP_URL/backup/status/$BACKUP_ID" backup 60)"
STORAGE_PATH="$(printf '%s' "$BACKUP_STATE" | python3 -c 'import sys,json;print(json.load(sys.stdin)["storage_path"])')"
log "Backup stored at s3://backups/$STORAGE_PATH"

log "Mutating the DB after the backup (adding a second thread row)"
sql "CREATE thread SET title = 'post-backup';" >/dev/null
[[ "$(thread_count)" == "2" ]] || fail "Expected 2 thread rows before restore"

RESTORE_ID="r-$BACKUP_ID"
log "Restoring $BACKUP_ID as $RESTORE_ID"
curl -sf -X POST "$SSP_URL/backup/restore" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"restore_id\":\"$RESTORE_ID\",\"backup_id\":\"$BACKUP_ID\",\"project_slug\":\"$PROJECT_SLUG\",\"storage_path\":\"$STORAGE_PATH\"}" >/dev/null

poll_status "$SSP_URL/backup/restore/status/$RESTORE_ID" restore 120 >/dev/null

log "Verifying restore rolled the DB back to 1 thread row"
COUNT="$(thread_count)"
[[ "$COUNT" == "1" ]] || fail "Expected 1 thread row after restore, got $COUNT"

log "Verifying the circuit re-bootstrapped from the restored dump"
for _ in $(seq 1 30); do
  if [[ "$(circuit_thread_count)" == "1" ]]; then break; fi
  sleep 1
done
[[ "$(circuit_thread_count)" == "1" ]] || fail "Circuit should hold 1 thread row after restore"

HEALTH="$(curl -sf "$SSP_URL/health" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])')"
[[ "$HEALTH" == "healthy" || "$HEALTH" == "ready" ]] || fail "SSP not healthy after restore: $HEALTH"

printf '\n\033[32mPASS: standalone SSP backup + restore round-trip succeeded\033[0m\n'
printf '\033[32m       (auth-gated /backup/*, main DB re-imported, circuit re-bootstrapped)\033[0m\n'
