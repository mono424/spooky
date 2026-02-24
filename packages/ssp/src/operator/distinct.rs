use crate::algebra::{ZSet, ZSetOps};
use crate::circuit::store::Store;
use crate::types::SpookyValue;
use std::collections::HashMap;

/// Distinct operator: ensures output weights are 0 or 1.
///
/// DBSP rule: `distinct = D(threshold(I(input)))`
/// where `threshold` clamps weights to {0, 1}.
///
/// On each step:
///   1. integrated += delta_in       (I: integration)
///   2. new_output = threshold(integrated)  (clamp to 0/1)
///   3. delta_out = new_output - prev_output (D: differentiation)
///   4. prev_output = new_output
#[derive(Debug)]
pub struct Distinct {
    /// Z⁻¹: accumulated input state.
    integrated: ZSet,
    /// Previous thresholded output (for differentiation).
    prev_output: ZSet,
}

impl Distinct {
    pub fn new() -> Self {
        Self {
            integrated: HashMap::new(),
            prev_output: HashMap::new(),
        }
    }

    fn threshold(zset: &ZSet) -> ZSet {
        zset.iter()
            .filter(|(_, &w)| w > 0)
            .map(|(k, _)| (k.clone(), 1i64))
            .collect()
    }
}

impl super::Operator for Distinct {
    fn snapshot(&self, inputs: &[&ZSet], _store: &Store, _ctx: Option<&SpookyValue>) -> ZSet {
        Self::threshold(inputs[0])
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        _store: &Store,
        _ctx: Option<&SpookyValue>,
    ) -> ZSet {
        // I: integrate input
        self.integrated.add(input_deltas[0]);

        // threshold: clamp to {0, 1}
        let new_output = Self::threshold(&self.integrated);

        // D: differentiate output
        let delta_out = self.prev_output.diff(&new_output);

        // Update state for next step
        self.prev_output = new_output;

        delta_out
    }

    fn arity(&self) -> usize {
        1
    }

    fn reset(&mut self) {
        self.integrated.clear();
        self.prev_output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn step_clamps_to_binary_presence() {
        let store = Store::new();
        let mut distinct = Distinct::new();

        // Insert with weight 3 → should appear with weight 1
        let d1 = zset(&[("a", 3)]);
        let result = distinct.step(&[&d1], &store, None);
        assert_eq!(result.get("a"), Some(&1));
    }

    #[test]
    fn step_emits_removal_when_weight_drops_to_zero() {
        let store = Store::new();
        let mut distinct = Distinct::new();

        let d1 = zset(&[("a", 2)]);
        let _ = distinct.step(&[&d1], &store, None);

        // Remove 2 → weight goes to 0 → should emit -1
        let d2 = zset(&[("a", -2)]);
        let result = distinct.step(&[&d2], &store, None);
        assert_eq!(result.get("a"), Some(&-1));
    }

    #[test]
    fn step_no_output_for_multiplicity_change_within_positive() {
        let store = Store::new();
        let mut distinct = Distinct::new();

        let d1 = zset(&[("a", 1)]);
        let _ = distinct.step(&[&d1], &store, None); // a enters

        let d2 = zset(&[("a", 2)]);
        let result = distinct.step(&[&d2], &store, None); // weight 1→3, threshold unchanged
        assert!(result.is_empty());
    }
}
