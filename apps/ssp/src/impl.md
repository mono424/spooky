# Implementation Plan: Edge Synchronization Fixes

## Overview

This plan addresses the issue where edges in `_spooky_list_ref` are incorrectly deleted when registering views or ingesting records. The fix involves three main areas:

1. **Delta computation fix** - Ensure first-run provides proper delta
2. **Record ID format alignment** - Match IDs between SSP and SurrealDB
3. **Re-registration cleanup** - Properly handle view re-registration

---

## Project Structure Reference

```
app/
└── ssp/
    └── src/
        └── lib.rs              # Server handlers (register_view, ingest, update_all_edges)

packages/
└── ssp/
    └── src/
        ├── circuit.rs          # Circuit, Database, Table, register_view logic
        ├── view.rs             # View, process_batch, process_delta, build_result_data
        └── update.rs           # ViewUpdate, StreamingUpdate, build_update, ViewDelta
```

---

## Phase 1: Add Diagnostic Logging (Day 1)

**Goal:** Understand the exact data flow and identify the root cause.

### Step 1.1: Add logging to `view.rs::process_batch`

**File:** `packages/ssp/src/view.rs`

**Location:** `process_batch` method (around line 395)

```rust
pub fn process_batch(
    &mut self,
    batch_deltas: &BatchDeltas,
    db: &Database,
) -> Option<ViewUpdate> {
    let is_first_run = self.last_hash.is_empty();

    // ADD THIS LOGGING BLOCK
    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        is_first_run = is_first_run,
        cache_size_before = self.cache.len(),
        last_hash = %self.last_hash,
        "Starting process_batch"
    );

    let view_delta = self.compute_view_delta(&batch_deltas.membership, db, is_first_run);
    let updated_record_ids = self.get_content_updates_in_view(batch_deltas);

    // ADD THIS LOGGING BLOCK
    let delta_additions: Vec<_> = view_delta.iter()
        .filter(|(_, w)| **w > 0)
        .map(|(k, _)| k.as_str())
        .collect();
    let delta_removals: Vec<_> = view_delta.iter()
        .filter(|(_, w)| **w < 0)
        .map(|(k, _)| k.as_str())
        .collect();
    
    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        additions_count = delta_additions.len(),
        removals_count = delta_removals.len(),
        additions_sample = ?delta_additions.iter().take(5).collect::<Vec<_>>(),
        removals_sample = ?delta_removals.iter().take(5).collect::<Vec<_>>(),
        "Computed view delta (keys have table prefix like 'user:123')"
    );

    // ... rest of method
}
```

### Step 1.2: Add logging to `lib.rs::update_all_edges`

**File:** `app/ssp/src/lib.rs`

**Location:** `update_all_edges` function (around line 534)

```rust
pub async fn update_all_edges<C: Connection>(
    db: &Surreal<C>, 
    updates: &[&StreamingUpdate], 
    metrics: &Metrics
) {
    // ADD THIS LOGGING BLOCK AT START
    for update in updates.iter() {
        let created: Vec<_> = update.records.iter()
            .filter(|r| matches!(r.event, DeltaEvent::Created))
            .map(|r| r.id.as_str())
            .collect();
        
        tracing::info!(
            target: "ssp::edges",
            view_id = %update.view_id,
            created_count = created.len(),
            created_ids = ?created.iter().take(10).collect::<Vec<_>>(),
            "StreamingUpdate record IDs (these are used in SQL queries)"
        );
    }

    // ... rest of function
}
```

### Step 1.3: Deploy and Test

```bash
# Set log level to see debug output
export RUST_LOG=ssp::view::process_batch=debug,ssp::edges=info

# Run the server
cargo run

# In another terminal, register a view and watch logs
curl -X POST http://localhost:8667/view/register \
  -H "Authorization: Bearer $SPOOKY_AUTH_SECRET" \
  -H "Content-Type: application/json" \
  -d '{ ... your view payload ... }'
```

### Step 1.4: Verify Record ID Format

Run this query in SurrealDB to check the format:

```sql
-- Check what format _spooky_version uses
SELECT id, record_id FROM _spooky_version LIMIT 5;

-- Expected output will show either:
-- record_id: "user:123"  (full format)
-- record_id: "123"       (stripped format)
```

**Document the result before proceeding!**

---

## Phase 2: Fix First-Run Delta (Day 1-2)

**Goal:** Ensure view registration emits proper delta for edge creation.

### Step 2.1: Modify `process_batch` to emit delta on first run

**File:** `packages/ssp/src/view.rs`

**Location:** Around line 424 in `process_batch`

**Before:**
```rust
let view_delta_struct = if is_first_run {
    None // First run = treat as full snapshot
} else {
    Some(ViewDelta {
        additions,
        removals,
        updates,
    })
};
```

**After:**
```rust
let view_delta_struct = if is_first_run {
    tracing::info!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        initial_records = additions.len(),
        "First run - emitting all records as additions"
    );
    // FIX: On first run, explicitly emit all records as additions
    // Previously None caused build_update to reconstruct from raw.records
    Some(ViewDelta {
        additions: additions.clone(),
        removals: vec![],
        updates: vec![],
    })
} else {
    Some(ViewDelta {
        additions,
        removals,
        updates,
    })
};
```

### Step 2.2: Unit Test

Add test to verify first-run behavior:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_run_emits_additions() {
        // Setup: Create view with empty cache
        let plan = QueryPlan { 
            id: "test".to_string(), 
            root: Operator::Scan { table: "users".to_string() } 
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Setup: Create database with one record
        let mut db = Database::new();
        let table = db.ensure_table("users");
        table.rows.insert(SmolStr::new("1"), SpookyValue::Null);
        table.zset.insert(SmolStr::new("users:1"), 1);
        
        // Act: Process empty batch (simulates first run on registration)
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        // Assert: Should return StreamingUpdate with Created event
        assert!(result.is_some());
        if let Some(ViewUpdate::Streaming(update)) = result {
            assert_eq!(update.records.len(), 1);
            assert!(matches!(update.records[0].event, DeltaEvent::Created));
        } else {
            panic!("Expected Streaming update");
        }
    }
}
```

---

## Phase 3: Fix Record ID Format (Day 2)

**Goal:** Align record ID format between SSP and SurrealDB.

### Step 3.1: Determine the correct approach

Based on Phase 1.4 findings, choose ONE of these approaches:

#### Option A: If `_spooky_version.record_id` uses FULL format (`user:123`)

**File:** `packages/ssp/src/view.rs`

**Location:** `build_result_data` method (around line 667)

**Change:** Keep full ID with table prefix

```rust
fn build_result_data(&self) -> Vec<SmolStr> {
    // CHANGED: Keep full ZSet key format (table:id)
    let mut result_data: Vec<SmolStr> = self.cache.keys().cloned().collect();
    result_data.sort_unstable();
    result_data
}
```

Also update `categorize_changes` to NOT strip prefix:

```rust
fn categorize_changes(...) -> (Vec<SmolStr>, Vec<SmolStr>, Vec<SmolStr>) {
    // ... 
    for (key, weight) in view_delta {
        if *weight > 0 {
            // CHANGED: Don't strip prefix
            additions.push(key.clone());
        } else if *weight < 0 {
            // CHANGED: Don't strip prefix
            removals.push(key.clone());
        }
    }
    // ...
}
```

#### Option B: If `_spooky_version.record_id` uses STRIPPED format (`123`)

Keep `build_result_data` as-is (stripping prefix), but update the SQL query to handle this:

**File:** `app/ssp/src/lib.rs`

**Location:** `update_all_edges` RELATE statement

```rust
DeltaEvent::Created => {
    format!(
        r#"LET $target = (SELECT id FROM _spooky_version WHERE record_id = '{0}');
        IF $target THEN
            RELATE ${1}->_spooky_list_ref->$target[0].id 
                SET version = $target[0].version, 
                    clientId = (SELECT clientId FROM ONLY ${1}).clientId
        ELSE
            RETURN {{ status: 'skipped', reason: 'no_version', record_id: '{0}' }}
        END"#,
        record.id,  // Already stripped: "123"
        binding_name,
    )
}
```

### Step 3.2: Update all SQL queries consistently

Ensure UPDATE and DELETE also use correct format:

```rust
DeltaEvent::Updated => {
    format!(
        "UPDATE ${1}->_spooky_list_ref SET version += 1 
         WHERE out = (SELECT id FROM _spooky_version WHERE record_id = '{0}')[0]",
        record.id,
        binding_name
    )
}

DeltaEvent::Deleted => {
    format!(
        "DELETE ${1}->_spooky_list_ref 
         WHERE out = (SELECT id FROM _spooky_version WHERE record_id = '{0}')[0]",
        record.id,
        binding_name
    )
}
```

---

## Phase 4: Fix Re-registration Cleanup (Day 2-3)

**Goal:** Properly clean up edges when re-registering a view.

### Step 4.1: Modify `register_view_handler`

**File:** `app/ssp/src/lib.rs`

**Location:** `register_view_handler` function (around line 366)

```rust
#[instrument(skip(state), fields(view_id = Empty))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let span = Span::current();

    let result = ssp::service::view::prepare_registration(payload);
    let data = match result {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    span.record("view_id", &data.plan.id);
    
    let m = &data.metadata;
    let raw_id = m["id"].as_str().unwrap();
    let id_str = format_incantation_id(raw_id);

    // NEW: Check if view exists and clean up old edges
    let view_existed = {
        let circuit = state.processor.read().await;
        circuit.views.iter().any(|v| v.plan.id == data.plan.id)
    };

    if view_existed {
        info!(
            target: "ssp::edges",
            view_id = %id_str,
            "Re-registering view - deleting old edges first"
        );
        
        if let Some(from_id) = parse_record_id(&id_str) {
            match state.db
                .query("DELETE $from->_spooky_list_ref RETURN BEFORE")
                .bind(("from", from_id))
                .await
            {
                Ok(mut response) => {
                    let deleted: Vec<Value> = response.take(0).unwrap_or_default();
                    info!(
                        target: "ssp::edges",
                        view_id = %id_str,
                        deleted_edge_count = deleted.len(),
                        "Cleaned up old edges"
                    );
                }
                Err(e) => {
                    error!(
                        target: "ssp::edges",
                        view_id = %id_str,
                        error = %e,
                        "Failed to delete old edges"
                    );
                }
            }
        }
        
        // Adjust metrics since we're replacing, not adding
        state.metrics.view_count.add(-1, &[]);
    }

    // Continue with normal registration...
    let update = {
        let mut circuit = state.processor.write().await;
        let res = circuit.register_view(
            data.plan.clone(),
            data.safe_params,
            Some(ViewResultFormat::Streaming),
        );
        state.saver.trigger_save();
        res
    };

    state.metrics.view_count.add(1, &[]);

    // ... rest of handler
}
```

---

## Phase 5: Validation & Testing (Day 3)

### Step 5.1: Manual Test Scenarios

Run these scenarios and verify logs:

#### Scenario A: Fresh View Registration

```bash
# 1. Ensure clean state
curl -X POST http://localhost:8667/reset -H "Authorization: Bearer $SECRET"

# 2. Ingest a record
curl -X POST http://localhost:8667/ingest -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{"table":"users","op":"CREATE","id":"user:1","record":{"name":"Alice"}}'

# 3. Register view that matches this record
curl -X POST http://localhost:8667/view/register -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{ ... view for users table ... }'

# 4. Verify edge exists
surreal sql --conn http://localhost:8000 --ns test --db test \
  "SELECT * FROM _spooky_list_ref"
```

**Expected logs:**
```
INFO ssp::view::process_batch: First run - emitting all records as additions view_id=... initial_records=1
INFO ssp::edges: StreamingUpdate record IDs view_id=... created_count=1 created_ids=["1"]
INFO ssp::edges: Edge transaction completed successfully statement_count=1
```

#### Scenario B: Record Ingestion After View Exists

```bash
# Ingest another record
curl -X POST http://localhost:8667/ingest -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{"table":"users","op":"CREATE","id":"user:2","record":{"name":"Bob"}}'

# Verify second edge exists
surreal sql "SELECT * FROM _spooky_list_ref"
```

**Expected:** Two edges exist, first edge unchanged.

#### Scenario C: View Re-registration

```bash
# Re-register same view
curl -X POST http://localhost:8667/view/register -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{ ... same view ... }'

# Verify edges recreated
surreal sql "SELECT * FROM _spooky_list_ref"
```

**Expected logs:**
```
INFO ssp::edges: Re-registering view - deleting old edges first view_id=...
INFO ssp::edges: Cleaned up old edges view_id=... deleted_edge_count=2
INFO ssp::edges: Edge transaction completed successfully statement_count=2
```

#### Scenario D: Multiple Views Sharing Records

```bash
# Register second view that also matches users
curl -X POST http://localhost:8667/view/register -H "Authorization: Bearer $SECRET" \
  -H "Content-Type: application/json" \
  -d '{ ... second view for users ... }'

# Verify edges from BOTH views exist
surreal sql "SELECT *, <-_spooky_incantation AS from_view FROM _spooky_list_ref"
```

**Expected:** 4 edges total (2 records × 2 views), each with correct `from_view`.

### Step 5.2: Automated Integration Test

```rust
#[tokio::test]
async fn test_edge_sync_lifecycle() {
    // Setup test server and DB
    let app = setup_test_app().await;
    
    // 1. Ingest record
    let resp = app.ingest("users", "CREATE", "user:1", json!({"name": "Test"})).await;
    assert_eq!(resp.status(), 200);
    
    // 2. Register view
    let resp = app.register_view("test-view", "SELECT * FROM users").await;
    assert_eq!(resp.status(), 200);
    
    // 3. Verify edge created
    let edges = app.query_db("SELECT * FROM _spooky_list_ref").await;
    assert_eq!(edges.len(), 1);
    
    // 4. Ingest second record
    app.ingest("users", "CREATE", "user:2", json!({"name": "Test2"})).await;
    
    // 5. Verify second edge created, first unchanged
    let edges = app.query_db("SELECT * FROM _spooky_list_ref").await;
    assert_eq!(edges.len(), 2);
    
    // 6. Re-register view
    app.register_view("test-view", "SELECT * FROM users").await;
    
    // 7. Verify edges recreated (not duplicated)
    let edges = app.query_db("SELECT * FROM _spooky_list_ref").await;
    assert_eq!(edges.len(), 2);
}
```

---

## Phase 6: Documentation & Cleanup (Day 3)

### Step 6.1: Update Code Comments

Add comments explaining the fix:

```rust
// In view.rs
/// Optimized 2-Phase Processing: Handles multiple table updates at once.
/// 
/// # Edge Sync Behavior
/// - First run (empty last_hash): Emits ALL matching records as additions
///   so that edges are created in SurrealDB
/// - Subsequent runs: Emits only changes (additions/removals/updates)
/// 
/// # Record ID Format
/// - Cache keys use ZSet format: "table:id" (e.g., "users:123")
/// - Result data uses stripped format: "123"
/// - This matches _spooky_version.record_id format
pub fn process_batch(...) { ... }
```

### Step 6.2: Remove Excessive Debug Logging

After verification, reduce logging to info/warn level for production:

```rust
// Change debug! to trace! for high-frequency logs
tracing::trace!(
    target: "ssp::view::process_batch",
    view_id = %self.plan.id,
    additions_sample = ?delta_additions.iter().take(5).collect::<Vec<_>>(),
    "Computed view delta"
);
```

### Step 6.3: Update README/Docs

Document the edge sync architecture and troubleshooting steps.

---

## Rollback Plan

If issues occur after deployment:

1. **Immediate:** Revert to previous code version
2. **Data Fix:** Run cleanup query:
   ```sql
   -- Delete all edges and let them be recreated
   DELETE _spooky_list_ref;
   ```
3. **Trigger Recreation:** Re-register all views to recreate edges

---

## Summary Checklist

- [ ] **Phase 1:** Add diagnostic logging
- [ ] **Phase 1:** Verify `_spooky_version.record_id` format
- [ ] **Phase 2:** Fix first-run delta emission
- [ ] **Phase 2:** Add unit test
- [ ] **Phase 3:** Align record ID format (Option A or B)
- [ ] **Phase 4:** Add re-registration cleanup
- [ ] **Phase 5:** Test Scenario A (fresh registration)
- [ ] **Phase 5:** Test Scenario B (ingest after view exists)
- [ ] **Phase 5:** Test Scenario C (re-registration)
- [ ] **Phase 5:** Test Scenario D (multi-view)
- [ ] **Phase 6:** Update comments
- [ ] **Phase 6:** Reduce logging verbosity
- [ ] **Phase 6:** Update documentation

---

## Appendix A: Complete Code Changes

### File 1: `packages/ssp/src/view.rs`

#### Change 1: Update `process_batch` method (around line 395)

**Replace the entire `process_batch` method with:**

```rust
/// Optimized 2-Phase Processing: Handles multiple table updates at once.
pub fn process_batch(
    &mut self,
    batch_deltas: &BatchDeltas,
    db: &Database,
) -> Option<ViewUpdate> {
    // FIX: FIRST RUN CHECK
    let is_first_run = self.last_hash.is_empty();

    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        is_first_run = is_first_run,
        cache_size_before = self.cache.len(),
        last_hash = %self.last_hash,
        "Starting process_batch"
    );

    // Compute view delta from membership changes
    let view_delta = self.compute_view_delta(&batch_deltas.membership, db, is_first_run);
    let updated_record_ids = self.get_content_updates_in_view(batch_deltas);

    let delta_additions: Vec<_> = view_delta.iter().filter(|(_, w)| **w > 0).map(|(k, _)| k.as_str()).collect();
    let delta_removals: Vec<_> = view_delta.iter().filter(|(_, w)| **w < 0).map(|(k, _)| k.as_str()).collect();
    
    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        delta_total = view_delta.len(),
        additions_count = delta_additions.len(),
        removals_count = delta_removals.len(),
        additions_sample = ?delta_additions.iter().take(5).collect::<Vec<_>>(),
        removals_sample = ?delta_removals.iter().take(5).collect::<Vec<_>>(),
        content_updates = updated_record_ids.len(),
        "Computed view delta (ZSet keys include table prefix)"
    );
    
    // Early return if no changes
    if view_delta.is_empty() && !is_first_run && updated_record_ids.is_empty() {
        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            "No changes detected, returning None"
        );
        return None;
    }

    // Apply delta to cache
    self.apply_cache_delta(&view_delta);

    // Categorize changes
    let (additions, removals, updates) = self.categorize_changes(&view_delta, &updated_record_ids);

    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        cache_size_after = self.cache.len(),
        categorized_additions = additions.len(),
        categorized_removals = removals.len(),
        categorized_updates = updates.len(),
        additions_sample = ?additions.iter().take(5).collect::<Vec<_>>(),
        removals_sample = ?removals.iter().take(5).collect::<Vec<_>>(),
        "Categorized changes (IDs are STRIPPED of table prefix)"
    );

    // Build result data
    let result_data = self.build_result_data();

    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        result_data_count = result_data.len(),
        result_sample = ?result_data.iter().take(5).collect::<Vec<_>>(),
        "Built result_data (these IDs go to StreamingUpdate)"
    );

    // Delegate formatting to update module (Strategy Pattern)
    use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};

    // FIX: On first run, we should still provide a delta for edge creation!
    // Previously this was None, causing build_update to use raw.records as Created
    // Now we explicitly create a delta with all records as additions
    let view_delta_struct = if is_first_run {
        tracing::info!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            initial_records = additions.len(),
            "First run - creating delta with all records as additions"
        );
        // On first run, all records in result are additions
        Some(ViewDelta {
            additions: additions.clone(),
            removals: vec![],
            updates: vec![],
        })
    } else {
        Some(ViewDelta {
            additions,
            removals,
            updates,
        })
    };

    // Compute hash if needed (for Streaming) before moving result_data
    let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
        Some(compute_flat_hash(&result_data))
    } else {
        None
    };

    let raw_result = RawViewResult {
        query_id: self.plan.id.clone(),
        records: result_data,
        delta: view_delta_struct,
    };

    // Build update using the configured format
    let update = build_update(raw_result, self.format);

    // Extract hash for comparison (depends on format)
    let hash = match &update {
        ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
        ViewUpdate::Streaming(_) => pre_hash.unwrap_or_default(),
    };

    let has_changes = match &update {
        ViewUpdate::Streaming(s) => !s.records.is_empty(),
        _ => hash != self.last_hash,
    };

    if has_changes {
        self.last_hash = hash;
        return Some(update);
    }

    None
}
```

---

### File 2: `app/ssp/src/lib.rs`

#### Change 1: Update `register_view_handler` (around line 366)

**Replace the entire `register_view_handler` function with:**

```rust
#[instrument(skip(state), fields(view_id = Empty))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let span = Span::current();

    let result = ssp::service::view::prepare_registration(payload);
    let data = match result {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    span.record("view_id", &data.plan.id);
    debug!("Registering view {}", data.plan.id);

    let m = &data.metadata;
    let raw_id = m["id"].as_str().unwrap();
    let id_str = format_incantation_id(raw_id);

    // Check if view already exists and delete its edges BEFORE re-registering
    // This ensures clean state when re-registering a view
    let view_existed = {
        let circuit = state.processor.read().await;
        circuit.views.iter().any(|v| v.plan.id == data.plan.id)
    };

    if view_existed {
        info!(
            target: "ssp::edges",
            view_id = %id_str,
            "View already exists, cleaning up old edges before re-registration"
        );
        if let Some(from_id) = parse_record_id(&id_str) {
            match state.db
                .query("DELETE $from->_spooky_list_ref RETURN BEFORE")
                .bind(("from", from_id))
                .await
            {
                Ok(mut response) => {
                    let deleted: Vec<Value> = response.take(0).unwrap_or_default();
                    info!(
                        target: "ssp::edges",
                        view_id = %id_str,
                        deleted_count = deleted.len(),
                        "Deleted old edges"
                    );
                }
                Err(e) => {
                    error!(
                        target: "ssp::edges",
                        view_id = %id_str,
                        error = %e,
                        "Failed to delete old edges"
                    );
                }
            }
        }
        // Decrement view count since register_view will add it back
        state.metrics.view_count.add(-1, &[]);
    }

    // Always register with Streaming mode
    let update = {
        let mut circuit = state.processor.write().await;
        let res = circuit.register_view(
            data.plan.clone(),
            data.safe_params,
            Some(ViewResultFormat::Streaming),
        );
        state.saver.trigger_save();
        res
    };

    state.metrics.view_count.add(1, &[]);

    let client_id_str = m["clientId"].as_str().unwrap().to_string();
    let surql_str = m["surrealQL"].as_str().unwrap().to_string();
    let ttl_str = m["ttl"].as_str().unwrap().to_string();
    let last_active_str = m["lastActiveAt"].as_str().unwrap().to_string();
    let params_val = m["safe_params"].clone();

    // Store incantation metadata
    let query = "UPSERT <record>$id SET clientId = <string>$clientId, surrealQL = <string>$surrealQL, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";

    let db_res = state
        .db
        .query(query)
        .bind(("id", id_str.clone()))
        .bind(("clientId", client_id_str))
        .bind(("surrealQL", surql_str))
        .bind(("params", params_val))
        .bind(("ttl", ttl_str))
        .bind(("lastActiveAt", last_active_str))
        .await;

    if let Err(e) = db_res {
        error!("Failed to upsert incantation metadata: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "DB Error").into_response();
    }

    // Create initial edges
    if let Some(ViewUpdate::Streaming(s)) = &update {
        info!(
            target: "ssp::edges",
            view_id = %id_str,
            record_count = s.records.len(),
            "Creating initial edges for view"
        );
        update_incantation_edges(&state.db, s, &state.metrics).await;
    }

    StatusCode::OK.into_response()
}
```

#### Change 2: Update `update_all_edges` function (around line 534)

**Replace the entire `update_all_edges` function with:**

```rust
/// Update edges for multiple views in a SINGLE transaction
/// Optimizes: 3 views × 1 record = 1 transaction instead of 3
#[instrument(skip(db, updates, metrics), fields(total_operations = Empty))]
pub async fn update_all_edges<C: Connection>(db: &Surreal<C>, updates: &[&StreamingUpdate], metrics: &Metrics) {
    if updates.is_empty() {
        return;
    }

    let span = Span::current();
    
    // Detailed logging of what we're about to process
    info!(
        target: "ssp::edges",
        update_count = updates.len(),
        total_records = updates.iter().map(|u| u.records.len()).sum::<usize>(),
        "update_all_edges called"
    );
    
    for update in updates.iter() {
        let created: Vec<_> = update.records.iter().filter(|r| matches!(r.event, DeltaEvent::Created)).map(|r| r.id.as_str()).collect();
        let updated: Vec<_> = update.records.iter().filter(|r| matches!(r.event, DeltaEvent::Updated)).map(|r| r.id.as_str()).collect();
        let deleted: Vec<_> = update.records.iter().filter(|r| matches!(r.event, DeltaEvent::Deleted)).map(|r| r.id.as_str()).collect();
        
        info!(
            target: "ssp::edges",
            view_id = %update.view_id,
            created_count = created.len(),
            updated_count = updated.len(), 
            deleted_count = deleted.len(),
            created_ids = ?created.iter().take(10).collect::<Vec<_>>(),
            updated_ids = ?updated.iter().take(10).collect::<Vec<_>>(),
            deleted_ids = ?deleted.iter().take(10).collect::<Vec<_>>(),
            "StreamingUpdate details (record IDs as received)"
        );
    }

    debug!(target: "ssp::edges", "view_edges: {}, records: {}", updates.len(), updates.iter().map(|u| u.records.len()).sum::<usize>());
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();

    let mut created_count = 0;
    let mut updated_count = 0;
    let mut deleted_count = 0;

    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() {
            continue;
        }

        let incantation_id_str = format_incantation_id(&update.view_id);
        debug!(target: "ssp::edges", "Incantation ID: {}", incantation_id_str);

        let Some(from_id) = parse_record_id(&incantation_id_str) else {
            error!("Invalid incantation ID format: {}", incantation_id_str);
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id.clone()));

        for record in &update.records {
            if parse_record_id(&record.id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %record.id,
                    view_id = %update.view_id,
                    "Invalid record ID format - cannot parse as RecordId"
                );
                continue;
            }
            
            debug!(
                target: "ssp::edges",
                view_id = %update.view_id,
                record_id = %record.id,
                event = ?record.event,
                "Processing record for edge operation"
            );

            let stmt = match record.event {
                DeltaEvent::Created => {
                    created_count += 1;
                    // Only create edge if _spooky_version record exists
                    // IMPORTANT: record.id is the STRIPPED id (e.g., "123" not "user:123")
                    // Make sure _spooky_version.record_id matches this format!
                    format!(
                        r#"LET $target = (SELECT id, record_id FROM _spooky_version WHERE record_id = '{0}');
                        IF $target THEN
                            RELATE ${1}->_spooky_list_ref->$target.id 
                                SET version = $target.version, 
                                    clientId = (SELECT clientId FROM ONLY ${1}).clientId;
                        ELSE
                            RETURN {{ status: 'skipped', reason: 'no_version_record', record_id: '{0}', view_id: '{2}' }};
                        END"#,
                        record.id,
                        binding_name,
                        update.view_id,
                    )
                }
                DeltaEvent::Updated => {
                    updated_count += 1;
                    format!(
                        "UPDATE ${1}->_spooky_list_ref SET version += 1 WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = '{0}')",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Deleted => {
                    deleted_count += 1;
                    format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = '{0}')",
                        record.id,
                        binding_name
                    )
                }
            };

            all_statements.push(stmt);
        }
    }

    if all_statements.is_empty() {
        return;
    }

    span.record("total_operations", all_statements.len());

    metrics.edge_operations.add(
        created_count,
        &[opentelemetry::KeyValue::new("operation", "create")],
    );
    metrics.edge_operations.add(
        updated_count,
        &[opentelemetry::KeyValue::new("operation", "update")],
    );
    metrics.edge_operations.add(
        deleted_count,
        &[opentelemetry::KeyValue::new("operation", "delete")],
    );

    debug!(
        "Processing {} edge operations across {} views",
        all_statements.len(),
        updates.len()
    );

    // Wrap ALL statements in ONE transaction
    let full_query = format!(
        "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
        all_statements.join(";\n")
    );

    // Build query with all bindings
    let mut query = db.query(&full_query);
    let mut debug_query = full_query.clone();

    for (name, id) in bindings {
        // Create a string representation for debugging
        let id_str = format!("{:?}", id);
        debug_query = debug_query.replace(&format!("${}", name), &id_str);

        query = query.bind((name, id));
    }

    info!(
        target: "ssp::edges",
        statement_count = all_statements.len(),
        created = created_count,
        updated = updated_count,
        deleted = deleted_count,
        "Executing edge transaction"
    );
    
    debug!(target: "ssp::edges", query = %debug_query, "Full query");

    match query.await {
        Ok(mut response) => {
            info!(
                target: "ssp::edges",
                statement_count = all_statements.len(),
                "Edge transaction completed successfully"
            );
            
            // Try to extract any returned data for debugging
            // This helps identify if RELATE/UPDATE/DELETE actually affected rows
            for i in 0..all_statements.len() {
                match response.take::<Vec<Value>>(i) {
                    Ok(results) if !results.is_empty() => {
                        debug!(
                            target: "ssp::edges",
                            statement_index = i,
                            result_count = results.len(),
                            results = ?results.iter().take(3).collect::<Vec<_>>(),
                            "Statement returned data"
                        );
                    }
                    _ => {}
                }
            }
        }
        Err(e) => {
            error!(
                target: "ssp::edges",
                error = %e,
                query = %debug_query,
                "Edge transaction FAILED"
            );
        }
    }
}
```

---

### File 3: `packages/ssp/src/update.rs`

**No changes required** - The `build_update` function already handles the delta correctly when it's provided. The fix is in `view.rs` to always provide a delta (even on first run).

---

### File 4: `packages/ssp/src/circuit.rs`

**No changes required** - The `register_view` function works correctly. The fix for re-registration edge cleanup is in `lib.rs` at the handler level.

---

## Appendix B: Log Targets Reference

After implementing these changes, use these log targets:

```bash
# See all edge-related logs
export RUST_LOG=ssp::edges=info

# See view processing details
export RUST_LOG=ssp::view::process_batch=debug

# See everything
export RUST_LOG=ssp::edges=debug,ssp::view::process_batch=debug

# Production recommended
export RUST_LOG=ssp::edges=info,ssp::view::process_batch=warn
```

---

## Appendix C: Quick Verification Queries

Run these in SurrealDB to verify the fix:

```sql
-- Check _spooky_version record_id format
SELECT id, record_id FROM _spooky_version LIMIT 5;

-- Count edges per incantation
SELECT in AS incantation, count() AS edge_count 
FROM _spooky_list_ref 
GROUP BY in;

-- Check edges for a specific view
SELECT ->_spooky_list_ref->_spooky_version.record_id AS records 
FROM _spooky_incantation:YOUR_VIEW_ID;

-- Find orphaned edges (edges to non-existent versions)
SELECT * FROM _spooky_list_ref 
WHERE out NOT IN (SELECT id FROM _spooky_version);
```