// src/lib.rs

#[cfg(all(not(target_arch = "wasm32"), feature = "mimalloc"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod converter;
pub mod permission_inject;
pub mod sanitizer;
pub mod service;
pub mod size;

// DBSP-theoretic module structure
pub mod algebra;
pub mod types;
pub mod operator;
pub mod circuit;
pub mod eval;

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub use rayon::prelude::*;
