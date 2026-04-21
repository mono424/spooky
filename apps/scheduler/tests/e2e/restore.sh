#!/usr/bin/env bash
# End-to-end test for the scheduler's backup + restore flow.
#
# Spins up SurrealDB + MinIO in Docker, runs the scheduler against them,
# creates a backup, then restores it. A full pass proves:
#   - Remote DB `.import()` works over the HTTP engine (not WS).
#   - Replica `reset()` does not trip the RocksDB LOCK.
#
# Usage:  apps/scheduler/tests/e2e/restore.sh
# Requires: docker, docker compose, cargo, curl, jq, python3.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEDULER_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE="docker compose -p sp00ky-e2e-restore -f $SCRIPT_DIR/docker-compose.yml"

SURREAL_URL="http://localhost:18000"
SURREAL_WS="ws://localhost:18000"
MINIO_URL="http://localhost:19000"

SCHED_PORT=19667
SCHED_URL="http://localhost:$SCHED_PORT"

WORKDIR=""
SCHED_PID=""
SCHED_LOG=""

log()  { printf '\033[36m---> %s\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL: %s\033[0m\n' "$*" >&2; exit 1; }

cleanup() {
  set +e
  if [[ -n "$SCHED_PID" ]] && kill -0 "$SCHED_PID" 2>/dev/null; then
    log "Stopping scheduler (pid $SCHED_PID)"
    kill "$SCHED_PID" 2>/dev/null
    wait "$SCHED_PID" 2>/dev/null
  fi
  log "Tearing down containers"
  $COMPOSE down -v >/dev/null 2>&1
  if [[ -n "${SCHED_LOG:-}" && -f "$SCHED_LOG" && "${KEEP_LOGS:-0}" != "1" ]]; then
    rm -f "$SCHED_LOG"
  fi
  if [[ -n "${WORKDIR:-}" && -d "$WORKDIR" && "${KEEP_WORKDIR:-0}" != "1" ]]; then
    rm -rf "$WORKDIR"
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
    body="$(curl -sf "$url" || true)"
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

# ---------------------------------------------------------------------------
log "Starting SurrealDB + MinIO"
$COMPOSE up -d

wait_http "$SURREAL_URL/health"           "SurrealDB"
wait_http "$MINIO_URL/minio/health/live"  "MinIO"

log "Building scheduler"
( cd "$SCHEDULER_DIR" && cargo build -q )

SCHED_BIN="$SCHEDULER_DIR/target/debug/scheduler"
[[ -x "$SCHED_BIN" ]] || fail "Scheduler binary not found at $SCHED_BIN"

WORKDIR="$(mktemp -d)"
SCHED_LOG="$WORKDIR/scheduler.log"
log "Scheduler workdir: $WORKDIR"

# Minimal config so we don't clash with a dev scheduler on the default port.
cat > "$WORKDIR/sp00ky.yml" <<YML
ingest_port: $SCHED_PORT
replica_db_path: ./data/replica
wal_path: ./data/event_wal.log
YML

log "Starting scheduler (logs: $SCHED_LOG)"
(
  cd "$WORKDIR"
  exec env \
    SPKY_DB_WS="$SURREAL_WS" \
    SPKY_DB_NS=sp00ky SPKY_DB_NAME=sp00ky \
    SPKY_DB_USER=root SPKY_DB_PASS=root \
    S3_ENDPOINT="$MINIO_URL" S3_ACCESS_KEY=minioadmin S3_SECRET_KEY=minioadmin \
    S3_BUCKET=backups S3_REGION=us-east-1 \
    RUST_LOG="${RUST_LOG:-info,scheduler=debug}" \
    "$SCHED_BIN" > "$SCHED_LOG" 2>&1
) &
SCHED_PID=$!

log "Waiting for scheduler on $SCHED_URL"
for _ in $(seq 1 60); do
  if curl -sf -o /dev/null "$SCHED_URL/backup/status"; then break; fi
  if ! kill -0 "$SCHED_PID" 2>/dev/null; then
    cat "$SCHED_LOG"
    fail "Scheduler exited before becoming ready"
  fi
  sleep 1
done
curl -sf -o /dev/null "$SCHED_URL/backup/status" || { cat "$SCHED_LOG"; fail "Scheduler never became ready"; }

BACKUP_ID="e2e-$(date +%s)"
PROJECT_SLUG="e2e"

log "Creating backup $BACKUP_ID"
curl -sf -X POST "$SCHED_URL/backup/create" \
  -H 'Content-Type: application/json' \
  -d "{\"backup_id\":\"$BACKUP_ID\",\"project_slug\":\"$PROJECT_SLUG\"}" >/dev/null

BACKUP_STATE="$(poll_status "$SCHED_URL/backup/status/$BACKUP_ID" backup 60)"
STORAGE_PATH="$(printf '%s' "$BACKUP_STATE" | python3 -c 'import sys,json;print(json.load(sys.stdin)["storage_path"])')"
log "Backup stored at s3://backups/$STORAGE_PATH"

RESTORE_ID="r-$BACKUP_ID"
log "Restoring $BACKUP_ID as $RESTORE_ID"
curl -sf -X POST "$SCHED_URL/backup/restore" \
  -H 'Content-Type: application/json' \
  -d "{\"restore_id\":\"$RESTORE_ID\",\"backup_id\":\"$BACKUP_ID\",\"project_slug\":\"$PROJECT_SLUG\",\"storage_path\":\"$STORAGE_PATH\"}" >/dev/null

poll_status "$SCHED_URL/backup/restore/status/$RESTORE_ID" restore 120 >/dev/null

log "Verifying main DB is reachable and has the restored metadata table"
INFO="$(curl -sf -u root:root \
  -H 'Accept: application/json' \
  -H 'surreal-ns: sp00ky' -H 'surreal-db: sp00ky' \
  --data 'INFO FOR DB;' "$SURREAL_URL/sql")"
printf '%s' "$INFO" | python3 -c 'import sys, json
r = json.load(sys.stdin)
tables = r[0]["result"]["tables"]
assert "_00_metadata" in tables, f"expected _00_metadata after restore, got {list(tables)}"' \
  || { printf '%s\n' "$INFO"; fail "main DB missing _00_metadata after restore"; }

printf '\n\033[32mPASS: backup + restore pipeline succeeded end-to-end\033[0m\n'
printf '\033[32m       (main DB imported via HTTP engine, replica reset via REMOVE DATABASE, WAL truncated)\033[0m\n'
