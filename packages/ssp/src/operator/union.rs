use crate::algebra::{ZSet, ZSetOps};
use crate::circuit::store::Store;
use crate::types::Sp00kyValue;
use std::collections::HashMap;

/// Union operator: stateless additive merge of two z-sets. `output = a + b`.
///
/// The DBSP delta rule for `+` is just `+`: `D(a+b) = dA + dB`. So `step` and
/// `snapshot` have the same body modulo input naming.
///
/// Used by the policy rewriter when lowering `OR` of permission branches over
/// the same outer scan: each branch's lowered subtree feeds into a Union, then
/// a `Distinct` collapses any duplicate left keys to weight 1. Without the
/// downstream `Distinct`, weights would multiply when a row matches multiple
/// branches — sometimes desirable, never for permissions.
#[derive(Debug, Default)]
pub struct Union;

impl Union {
    pub fn new() -> Self {
        Self
    }
}

impl super::Operator for Union {
    fn snapshot(&self, inputs: &[&ZSet], _store: &Store, _ctx: Option<&Sp00kyValue>) -> ZSet {
        let mut out: ZSet = HashMap::with_capacity(inputs[0].len() + inputs[1].len());
        out.add(inputs[0]);
        out.add(inputs[1]);
        out
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        ctx: Option<&Sp00kyValue>,
    ) -> ZSet {
        // Stateless: identical to snapshot.
        self.snapshot(input_deltas, store, ctx)
    }

    fn arity(&self) -> usize {
        2
    }

    fn reset(&mut self) {}

    fn evaluate_key(
        &self,
        _key: &str,
        input_evals: &[bool],
        _store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> bool {
        // Union admits the key if either branch admits it. This is
        // critical for the permission-lowering shape where multiple
        // OR-branches feed into a Union.
        input_evals.iter().any(|&e| e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| ((*k).into(), *w)).collect()
    }

    #[test]
    fn snapshot_sums_disjoint_inputs() {
        let store = Store::new();
        let u = Union::new();
        let a = zset(&[("x", 1)]);
        let b = zset(&[("y", 1)]);
        let r = u.snapshot(&[&a, &b], &store, None);
        assert_eq!(r.get("x"), Some(&1));
        assert_eq!(r.get("y"), Some(&1));
    }

    #[test]
    fn snapshot_sums_overlapping_inputs() {
        // When both branches contain the same key, weights add. Downstream
        // Distinct is responsible for clamping to {0,1} for permission OR.
        let store = Store::new();
        let u = Union::new();
        let a = zset(&[("x", 1)]);
        let b = zset(&[("x", 1)]);
        let r = u.snapshot(&[&a, &b], &store, None);
        assert_eq!(r.get("x"), Some(&2));
    }

    #[test]
    fn step_passes_through_negative_deltas() {
        let store = Store::new();
        let mut u = Union::new();
        let a = zset(&[("x", -1)]);
        let b = zset(&[("y", 1)]);
        let r = u.step(&[&a, &b], &store, None);
        assert_eq!(r.get("x"), Some(&-1));
        assert_eq!(r.get("y"), Some(&1));
    }

    #[test]
    fn step_matches_snapshot_diff() {
        let store = Store::new();
        let condition_a_before = zset(&[("x", 1)]);
        let condition_b_before = zset(&[("y", 1)]);
        let snap_before = Union::new().snapshot(
            &[&condition_a_before, &condition_b_before],
            &store,
            None,
        );

        let condition_a_after = zset(&[("x", 1), ("z", 1)]);
        let condition_b_after = zset(&[("y", 1), ("w", 1)]);
        let snap_after = Union::new().snapshot(
            &[&condition_a_after, &condition_b_after],
            &store,
            None,
        );
        let expected_delta = snap_before.diff(&snap_after);

        let mut u = Union::new();
        let dA = zset(&[("z", 1)]);
        let dB = zset(&[("w", 1)]);
        let actual_delta = u.step(&[&dA, &dB], &store, None);
        assert_eq!(actual_delta, expected_delta);
    }
}
