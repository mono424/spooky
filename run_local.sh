#!/bin/bash
export SP00KY_ENV=staging
export SPKY_DB_WS=wss://whitepawn-db.stg.spky.cloud
export SPKY_DB_USER=root
export SPKY_DB_PASS=$(curl -s -X POST https://api-stg.sp00ky.cloud/v1/projects/whitepawn/deployments/credentials -H "Authorization: Bearer spk_live_$(cat /Users/khadim/.sp00ky/credentials.json | jq -r .access_token | cut -d'_' -f3-)" | jq -r .db_token)
export SPKY_DB_NS=test
export SPKY_DB_NAME=test
export SPKY_SCHEDULER_LISTEN_ADDR=127.0.0.1:9000
export RUST_LOG=info,scheduler=trace

# Clean up previous local RocksDB
rm -rf .sp00ky_replica.db

cargo run -p scheduler
