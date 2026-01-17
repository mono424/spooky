/// Debug logging macro that only compiles for WASM targets.
/// Usage: debug_log!("message {} {}", var1, var2);
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!($($arg)*).into());
        }
    };
}

// Re-export for internal use
pub use debug_log;
