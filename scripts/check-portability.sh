#!/usr/bin/env bash
# Portability gate: the ssp-node core must always build for wasm32 (the
# Cloudflare Workers target). Run from anywhere; CI should run it on every
# change to packages/ssp-node or its dependencies.
#
# Invoked from the crate dir so packages/ssp-node/.cargo/config.toml applies
# (getrandom wasm_js backend cfg — same pattern as packages/ssp-wasm).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/../packages/ssp-node"

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo check --target wasm32-unknown-unknown "$@"
echo "OK: ssp-node builds for wasm32-unknown-unknown"
