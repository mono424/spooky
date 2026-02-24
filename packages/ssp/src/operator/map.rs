use crate::algebra::ZSet;
use crate::circuit::store::Store;
use crate::types::SpookyValue;

use super::plan::Projection;

/// Map operator: field projection / rename.
///
/// For non-subquery projections, this is a transparent pass-through
/// (keys are unchanged, only the projected fields change — but since
/// we track by key, not by value, the Z-set passes through as-is).
///
/// Stateless (arity 1). The delta rule is the identity:
/// `delta_out = delta_in`
#[derive(Debug)]
pub struct Map {
    pub projections: Vec<Projection>,
}

impl Map {
    pub fn new(projections: Vec<Projection>) -> Self {
        Self { projections }
    }
}

impl super::Operator for Map {
    fn snapshot(&self, inputs: &[&ZSet], _store: &Store, _ctx: Option<&SpookyValue>) -> ZSet {
        // Field projection doesn't change keys, only values.
        // Since we track membership by key, pass through.
        inputs[0].clone()
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        _store: &Store,
        _ctx: Option<&SpookyValue>,
    ) -> ZSet {
        input_deltas[0].clone()
    }

    fn arity(&self) -> usize {
        1
    }

    fn reset(&mut self) {}
}
