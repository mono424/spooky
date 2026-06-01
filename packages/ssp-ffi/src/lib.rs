//! C ABI for the `ssp` stream processor, for use from Dart FFI.
//!
//! Mirrors the surface that `ssp-wasm` exposes to JavaScript, but instead of
//! `wasm-bindgen` it passes UTF-8 JSON across a C boundary. Each fallible call
//! returns a freshly-allocated, NUL-terminated C string holding a JSON
//! envelope: `{"ok": <data>}` on success or `{"err": "<message>"}` on failure.
//!
//! ## Memory ownership
//! - Every `*mut c_char` returned by an `ssp_*` function is owned by this
//!   library's allocator. The caller MUST return it via [`ssp_string_free`]
//!   after copying out the bytes.
//! - Every `*const c_char` argument is owned by the caller; this library only
//!   borrows it for the duration of the call.
//! - The `*mut Processor` handle is owned by the caller and MUST be released
//!   with [`ssp_free`]. It is never freed by any other function.

mod processor;

use processor::Processor;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Borrow a `*const c_char` as `&str`, erroring on null / invalid UTF-8.
///
/// # Safety
/// `p` must be null or a valid NUL-terminated C string that outlives the call.
unsafe fn cstr<'a>(p: *const c_char) -> anyhow::Result<&'a str> {
    if p.is_null() {
        anyhow::bail!("null pointer argument");
    }
    Ok(CStr::from_ptr(p).to_str()?)
}

/// Allocate a JSON envelope C string. Never panics on encoding (the envelope
/// is always valid JSON without interior NULs).
fn envelope(value: Value) -> *mut c_char {
    let s = value.to_string();
    // A serde_json string never contains an interior NUL byte.
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{\"err\":\"interior nul in result\"}").unwrap())
        .into_raw()
}

/// Run `f`, catching panics, and serialize the outcome into a JSON envelope.
fn ffi_call(f: impl FnOnce() -> anyhow::Result<Value>) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(f));
    let value = match result {
        Ok(Ok(v)) => serde_json::json!({ "ok": v }),
        Ok(Err(e)) => serde_json::json!({ "err": e.to_string() }),
        Err(_) => serde_json::json!({ "err": "panic in ssp-ffi" }),
    };
    envelope(value)
}

/// Create a new processor. Caller must eventually call [`ssp_free`].
#[no_mangle]
pub extern "C" fn ssp_new() -> *mut Processor {
    match catch_unwind(|| Box::into_raw(Box::new(Processor::new()))) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a processor created by [`ssp_new`].
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`ssp_new`] that has not already
/// been freed.
#[no_mangle]
pub unsafe extern "C" fn ssp_free(ptr: *mut Processor) {
    if ptr.is_null() {
        return;
    }
    drop(Box::from_raw(ptr));
}

/// Free a string returned by any `ssp_*` function.
///
/// # Safety
/// `s` must be null or a pointer returned by this library that has not already
/// been freed.
#[no_mangle]
pub unsafe extern "C" fn ssp_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(CString::from_raw(s));
}

/// Ingest one record change. `record_json` is the JSON object for the record.
/// Returns `{"ok":[WasmViewUpdate,...]}` or `{"err":"..."}`.
///
/// # Safety
/// `ptr` must be a valid processor handle; the string args must be valid
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn ssp_ingest(
    ptr: *mut Processor,
    table: *const c_char,
    op: *const c_char,
    id: *const c_char,
    record_json: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &mut *ptr;
        let table = cstr(table)?;
        let op = cstr(op)?;
        let id = cstr(id)?;
        let record: Value = serde_json::from_str(cstr(record_json)?)?;
        let updates = p.ingest(table, op, id, record)?;
        Ok(serde_json::to_value(updates)?)
    })
}

/// Register a materialized view from a JSON config.
/// Returns `{"ok":WasmViewUpdate}` or `{"err":"..."}`.
///
/// # Safety
/// See [`ssp_ingest`].
#[no_mangle]
pub unsafe extern "C" fn ssp_register_view(
    ptr: *mut Processor,
    config_json: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &mut *ptr;
        let config: Value = serde_json::from_str(cstr(config_json)?)?;
        Ok(serde_json::to_value(p.register_view(config)?)?)
    })
}

/// Unregister a view by id. Returns `{"ok":null}` or `{"err":"..."}`.
///
/// # Safety
/// See [`ssp_ingest`].
#[no_mangle]
pub unsafe extern "C" fn ssp_unregister_view(
    ptr: *mut Processor,
    id: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &mut *ptr;
        p.unregister_view(cstr(id)?);
        Ok(Value::Null)
    })
}

/// Register a table's `PERMISSIONS FOR select WHERE <expr>` text on the
/// circuit. Returns `{"ok":null}` or `{"err":"..."}`.
///
/// # Safety
/// See [`ssp_ingest`].
#[no_mangle]
pub unsafe extern "C" fn ssp_set_permission(
    ptr: *mut Processor,
    table: *const c_char,
    where_text: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &mut *ptr;
        p.set_permission(cstr(table)?, cstr(where_text)?);
        Ok(Value::Null)
    })
}

/// Save circuit state. Returns `{"ok":"<state json>"}` or `{"err":"..."}`.
///
/// # Safety
/// `ptr` must be a valid processor handle.
#[no_mangle]
pub unsafe extern "C" fn ssp_save_state(ptr: *const Processor) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &*ptr;
        Ok(Value::String(p.save_state()?))
    })
}

/// Load circuit state. Returns `{"ok":null}` or `{"err":"..."}`.
///
/// # Safety
/// See [`ssp_ingest`].
#[no_mangle]
pub unsafe extern "C" fn ssp_load_state(
    ptr: *mut Processor,
    state: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        if ptr.is_null() {
            anyhow::bail!("null processor handle");
        }
        let p = &mut *ptr;
        p.load_state(cstr(state)?)?;
        Ok(Value::Null)
    })
}
