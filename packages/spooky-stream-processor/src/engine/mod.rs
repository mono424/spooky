pub mod circuit {
    pub use super::lazy_circuit::LazyCircuit as Circuit; // Alias for backward compatibility if needed, but we should be explicit
}
pub mod lazy_circuit;
pub mod standard_circuit;
pub mod store;
pub mod view;
