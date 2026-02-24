// src/lib.rs

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod converter;
pub mod sanitizer;
pub mod service;

// DBSP-theoretic module structure
pub mod algebra;
pub mod types;
pub mod operator;
pub mod circuit;
pub mod eval;

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub use rayon::prelude::*;
