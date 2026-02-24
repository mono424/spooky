#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCHEMA_DIR="$(cd "$SCRIPT_DIR/../../schema" && pwd)"

cd "$SCHEMA_DIR"

echo "[e2e] Building schema..."
pnpm build

echo "[e2e] Starting Docker services (sidecar mode)..."
docker compose -f docker-compose.sidecar.yml up --build --force-recreate
