# Analysis: lib.rs Edge Management Issues

## Overview

After reviewing the code, I've identified several issues that could cause "strange" behavior in real-world usage even when tests pass.

---

## Issue 1: CRITICAL - RELATE Creates Duplicate Edges

**Location:** Lines 646-652 (update_all_edges)

**Problem:** The `RELATE` statement creates a NEW edge every time it's called, even if an edge already exists between the same `from` and `to` nodes.

```sql
RELATE ${1}->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = {0}) 
    SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version, 
        clientId = (SELECT clientId FROM ONLY ${1}).clientId
```

**What Happens:**
1. View registers → Creates edge `view:1 -> user:1`
2. User content updates → SSP emits `Created` event (due to fast path bug or re-registration)
3. `RELATE` creates ANOTHER edge `view:1 -> user:1`
4. Now you have duplicate edges!

**Fix:** Use `RELATE ... ON DUPLICATE KEY UPDATE` or check existence first:

```sql
-- Option 1: Use IF NOT EXISTS (SurrealDB specific)
LET $to = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0});
IF (SELECT * FROM _spooky_list_ref WHERE in = ${1} AND out = $to) = [] THEN
    RELATE ${1}->_spooky_list_ref->$to 
        SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version,
            clientId = (SELECT clientId FROM ONLY ${1}).clientId;
END;

-- Option 2: Upsert pattern (if SurrealDB supports)
UPSERT INTO _spooky_list_ref ...
```

---

## Issue 2: CRITICAL - Record ID Format Mismatch

**Location:** Lines 637-640, 646-652

**Problem:** The `record.id` from `StreamingUpdate` is in format `"user:1"`, but your SQL expects to find it in `_spooky_version.record_id`.

```rust
// record.id = "user:1"
let stmt = format!(
    "RELATE ${1}->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = {0})",
    record.id,  // "user:1" - is this quoted? does _spooky_version have this exact format?
    binding_name,
)
```

**Questions:**
1. Is `record_id` in `_spooky_version` stored as `"user:1"` or `user:1` (RecordId type)?
2. Does the `SELECT id FROM ONLY _spooky_version WHERE record_id = user:1` work correctly?

**Debugging:** Add logging to see what the actual query looks like:

```rust
tracing::debug!(
    target: "ssp::edges",
    record_id = %record.id,
    statement = %stmt,
    "Generated RELATE statement"
);
```

**Potential Fix:** Quote the record ID properly:

```rust
// If record_id is stored as a string
format!(
    "RELATE ... WHERE record_id = '{0}'",  // Add quotes
    record.id,
)

// Or if it's a RecordId type
format!(
    "RELATE ... WHERE record_id = <record>{0}",
    record.id,
)
```

---

## Issue 3: SELECT id FROM ONLY Returns NONE

**Location:** Lines 646-652

**Problem:** `SELECT id FROM ONLY` returns NONE if no record exists, causing `RELATE` to fail silently.

```sql
RELATE $from->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = user:1)
```

If `_spooky_version` doesn't have a record for `user:1`:
- The subquery returns NONE
- RELATE with NONE as target fails or creates broken edge
- No error is raised!

**Fix:** Check for existence or handle NONE:

```sql
LET $target = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0});
IF $target != NONE THEN
    RELATE ${1}->_spooky_list_ref->$target SET ...;
ELSE
    -- Log or handle missing version record
END;
```

---

## Issue 4: Version Update Logic

**Location:** Lines 654-660

**Problem:** `SET version += 1` increments the edge version, but you probably want to sync it with `_spooky_version.version`:

```sql
-- Current: Increments edge version independently
UPDATE ${1}->_spooky_list_ref SET version += 1 WHERE out = ...

-- Should be: Sync with actual record version?
UPDATE ${1}->_spooky_list_ref 
    SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version
    WHERE out = ...
```

This depends on your requirements - do edges track their own version or mirror the record version?

---

## Issue 5: Re-Registration Edge Cleanup Race Condition

**Location:** Lines 389-415 (register_view_handler)

**Problem:** There's a race condition between deleting old edges and creating new ones:

```rust
// 1. Read lock - check if exists
let view_existed = {
    let circuit = state.processor.read().await;
    circuit.views.iter().any(|v| v.plan.id == data.plan.id)
};

// 2. Gap here - another request could modify!

if view_existed {
    // 3. Delete edges (no lock on circuit)
    state.db.query("DELETE $from->_spooky_list_ref").await;
}

// 4. Gap here - ingest could run!

// 5. Write lock - register view
let update = {
    let mut circuit = state.processor.write().await;
    circuit.register_view(...)
};

// 6. Create new edges
```

**Sequence of events that causes problems:**
1. Re-register starts, deletes edges
2. Ingest runs, view emits Created events
3. Ingest creates edges
4. Re-register completes, creates DUPLICATE edges

**Fix:** Use a single lock scope:

```rust
let update = {
    let mut circuit = state.processor.write().await;
    
    // Check and cleanup under write lock
    let view_existed = circuit.views.iter().any(|v| v.plan.id == data.plan.id);
    
    if view_existed {
        // Delete edges while holding lock
        if let Some(from_id) = parse_record_id(&id_str) {
            let _ = state.db
                .query("DELETE $from->_spooky_list_ref")
                .bind(("from", from_id))
                .await;
        }
    }
    
    // Register view
    let res = circuit.register_view(...);
    state.saver.trigger_save();
    res
};
```

---

## Issue 6: Transaction Error Handling

**Location:** Lines 721-732

**Problem:** If the transaction fails, you just log an error and continue:

```rust
match query.await {
    Ok(_) => { debug!(...); }
    Err(e) => { error!("Batched edge update failed: {}", e); }  // Just log!
}
```

**Impact:**
- Edges get out of sync with view cache
- No retry mechanism
- No way to recover

**Fix:** Add retry logic or mark view as needing full edge sync:

```rust
match query.await {
    Ok(_) => { debug!(...); }
    Err(e) => {
        error!("Batched edge update failed: {}", e);
        
        // Option 1: Retry
        // retry_edge_updates(db, updates).await;
        
        // Option 2: Mark view for full sync
        // mark_views_for_edge_resync(&updates);
        
        // Option 3: Return error to caller
        // return Err(EdgeUpdateError::TransactionFailed(e));
    }
}
```

---

## Issue 7: Missing parse_record_id Logging

**Location:** Lines 637-640

**Problem:** When `parse_record_id` fails, you log but continue, potentially skipping valid operations:

```rust
if parse_record_id(&record.id).is_none() {
    error!("Invalid record ID format: {}", record.id);
    continue;  // Skips this record entirely!
}
```

**Issue:** The error message doesn't tell you WHY the parse failed. Add more context:

```rust
if parse_record_id(&record.id).is_none() {
    error!(
        target: "ssp::edges",
        record_id = %record.id,
        view_id = %update.view_id,
        event = ?record.event,
        "Invalid record ID format - skipping edge operation"
    );
    continue;
}
```

---

## Issue 8: The `SELECT id FROM ONLY` Pattern

**Location:** Multiple places

**Problem:** `SELECT id FROM ONLY` with a `WHERE` clause might not work as expected in SurrealDB:

```sql
-- You're doing this:
SELECT id FROM ONLY _spooky_version WHERE record_id = user:1

-- But ONLY expects a specific record, not a filter:
SELECT * FROM ONLY user:1  -- This works
SELECT * FROM ONLY table WHERE x = y  -- This might not work as expected!
```

**Fix:** Use `SELECT VALUE id FROM _spooky_version WHERE ... LIMIT 1`:

```sql
SELECT VALUE id FROM _spooky_version WHERE record_id = 'user:1' LIMIT 1
```

---

## Debugging Recommendations

### 1. Add Debug Endpoint for Edge State

```rust
async fn debug_edges_handler(
    State(state): State<AppState>,
    Path(view_id): Path<String>,
) -> impl IntoResponse {
    let id_str = format_incantation_id(&view_id);
    
    let result = state.db
        .query("SELECT * FROM $from->_spooky_list_ref")
        .bind(("from", parse_record_id(&id_str)))
        .await;
    
    match result {
        Ok(mut res) => {
            let edges: Vec<Value> = res.take(0).unwrap_or_default();
            Json(json!({
                "view_id": view_id,
                "edge_count": edges.len(),
                "edges": edges,
            }))
        }
        Err(e) => Json(json!({ "error": e.to_string() }))
    }
}
```

### 2. Add Edge Count Verification

After each edge update, verify counts match:

```rust
async fn verify_edge_counts(
    db: &Surreal<Client>,
    view_id: &str,
    expected_count: usize,
) {
    let id_str = format_incantation_id(view_id);
    let result = db
        .query("SELECT count() FROM $from->_spooky_list_ref GROUP ALL")
        .bind(("from", parse_record_id(&id_str)))
        .await;
    
    if let Ok(mut res) = result {
        let actual: Option<usize> = res.take("count").ok();
        if actual != Some(expected_count) {
            error!(
                target: "ssp::edges",
                view_id = %view_id,
                expected = expected_count,
                actual = ?actual,
                "Edge count mismatch!"
            );
        }
    }
}
```

### 3. Log the Actual SQL

```rust
// Before executing
info!(
    target: "ssp::edges::sql",
    query = %debug_query,
    "Executing edge update transaction"
);
```

---

## Summary of Fixes

| Issue | Priority | Fix |
|-------|----------|-----|
| 1. RELATE creates duplicates | **CRITICAL** | Add duplicate check or use UPSERT |
| 2. Record ID format mismatch | **HIGH** | Verify format, add quotes if needed |
| 3. SELECT ONLY returns NONE | **HIGH** | Add existence check |
| 4. Version update logic | Medium | Clarify requirements |
| 5. Re-registration race | **HIGH** | Use single lock scope |
| 6. Transaction error handling | Medium | Add retry/recovery |
| 7. Missing parse_record_id logging | Low | Add context to errors |
| 8. SELECT FROM ONLY pattern | **HIGH** | Use SELECT VALUE ... LIMIT 1 |