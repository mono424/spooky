/// Debug logging macro that outputs to console for WASM and stderr for native.
/// Usage: debug_log!("message {} {}", var1, var2);
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        // 1. Tracing (Aspire / OTLP)
        // Escalated to INFO to ensure it's not filtered out by default env settings
        tracing::info!(target: "ssp_module", $($arg)*);

        // 2. Legacy / Frontend (Console / Stderr)
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!($($arg)*).into());
        }
    };
}

/// Returns a root span appropriate for the current architecture.
/// - Native: "ssp-module"
/// - WASM: "ssp-module-wasm"
pub fn get_module_span() -> tracing::Span {
    #[cfg(target_arch = "wasm32")]
    {
        tracing::info_span!("ssp_module_wasm")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tracing::info_span!("ssp_module")
    }
}

// Re-export for internal use
pub use debug_log;

