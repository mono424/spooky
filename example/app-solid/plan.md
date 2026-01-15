# Fixed: WASM Console Logging

## Problem

The debug logs weren't appearing because `println!` doesn't work in WASM for browser output. WASM needs to use `web_sys::console::log_1` to write to the browser console.

## Changes Made

### 1. Added `web-sys` dependency

**File:** `packages/spooky-stream-processor/Cargo.toml`

- Added `web-sys = { version = "0.3", features = ["console"] }` to WASM target dependencies

### 2. Replaced `println!` with `web_sys::console::log_1`

**File:** `packages/spooky-stream-processor/src/engine/circuit.rs`

- Replaced all `println!` statements with `web_sys::console::log_1(&format!(...).into())`
- This enables proper browser console output

### 3. Rebuilt packages

- ✅ Rebuilt WASM: `~/.cargo/bin/wasm-pack build --target web`
- ✅ Rebuilt core: `npm run build`

## Next Steps

**You need to restart your dev server and test again:**

1. **Stop the dev server** (Ctrl+C)

2. **Clear browser cache completely:**
   - Open DevTools (F12)
   - Application tab → Clear storage → Clear site data
   - Or hard reload: `Cmd + Shift + R` (Mac)

3. **Restart dev server:**

   ```bash
   cd /Users/khadim/dev/spooky/example/app-solid
   npm run dev
   ```

4. **Navigate to a thread detail page**

5. **Check console** - you should now see:

   ```
   DEBUG: Rebuilding dependency graph for X views
   DEBUG: View 0 (id: ...) depends on tables: [...]
   DEBUG: Final dependency graph: {...}
   ```

6. **Create a new comment**

7. **Check console** - you should see:
   ```
   DEBUG: Changed tables: ["comment"]
   DEBUG: Table comment impacts views: [...]
   DEBUG: Total impacted view indices (before dedup): [...]
   ```

## What the Logs Will Tell Us

The debug logs will reveal:

- **If `extract_tables` is working:** Does it find `["thread", "comment", "user"]` for the thread detail view?
- **If dependency graph is correct:** Does `"comment"` map to the thread detail view index?
- **If views are being triggered:** Is the thread detail view in `impacted_view_indices` when a comment is created?

Once we see the logs, we'll know exactly where the bug is!
