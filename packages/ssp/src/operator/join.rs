use crate::algebra::{ZSet, ZSetOps};
use crate::circuit::store::Store;
use crate::eval::value_ops::{compare_values, hash_value, resolve_field};
use crate::operator::plan::JoinCondition;
use crate::types::SpookyValue;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Join operator with Z⁻¹ integration state for both inputs.
///
/// DBSP delta rule for equi-join:
///   delta_out = (delta_A ⋈ state_B)
///             + (state_A ⋈ delta_B)
///             + (delta_A ⋈ delta_B)
///
/// On each `step()` call:
///   1. Compute the three join terms using stored state
///   2. Update state: state_A += delta_A, state_B += delta_B
///   3. Return the sum of the three terms
#[derive(Debug)]
pub struct Join {
    pub condition: JoinCondition,
    /// Z⁻¹ accumulated state for left input.
    pub left_state: ZSet,
    /// Z⁻¹ accumulated state for right input.
    pub right_state: ZSet,
}

impl Join {
    pub fn new(condition: JoinCondition) -> Self {
        Self {
            condition,
            left_state: HashMap::new(),
            right_state: HashMap::new(),
        }
    }

    /// Hash-join: probe left against right using the join condition.
    /// Output weight = left_weight * right_weight (ring multiplication).
    fn hash_join(left: &ZSet, right: &ZSet, condition: &JoinCondition, store: &Store) -> ZSet {
        if left.is_empty() || right.is_empty() {
            return HashMap::new();
        }

        // Build index on the right side
        let mut right_index: HashMap<u64, Vec<(&String, &i64, &SpookyValue)>> = HashMap::new();
        for (r_key, r_weight) in right {
            if let Some(r_val) = store.get_row_by_key(r_key) {
                if let Some(r_field) = resolve_field(Some(r_val), &condition.right_field) {
                    let hash = hash_value(r_field);
                    right_index
                        .entry(hash)
                        .or_default()
                        .push((r_key, r_weight, r_field));
                }
            }
        }

        // Probe from the left side
        let mut out = HashMap::new();
        for (l_key, &l_weight) in left {
            if let Some(l_val) = store.get_row_by_key(l_key) {
                if let Some(l_field) = resolve_field(Some(l_val), &condition.left_field) {
                    let hash = hash_value(l_field);
                    if let Some(matches) = right_index.get(&hash) {
                        for (_r_key, &r_weight, r_field) in matches {
                            if compare_values(Some(l_field), Some(r_field)) == Ordering::Equal {
                                let w = l_weight * r_weight;
                                *out.entry(l_key.clone()).or_insert(0) += w;
                            }
                        }
                    }
                }
            }
        }

        // Clean up zero weights
        out.retain(|_, w| *w != 0);
        out
    }
}

impl super::Operator for Join {
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, _ctx: Option<&SpookyValue>) -> ZSet {
        Self::hash_join(inputs[0], inputs[1], &self.condition, store)
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        _ctx: Option<&SpookyValue>,
    ) -> ZSet {
        let delta_a = input_deltas[0];
        let delta_b = input_deltas[1];

        // Three-term join delta rule
        let term1 = Self::hash_join(delta_a, &self.right_state, &self.condition, store);
        let term2 = Self::hash_join(&self.left_state, delta_b, &self.condition, store);
        let term3 = Self::hash_join(delta_a, delta_b, &self.condition, store);

        // Update Z⁻¹ integration state AFTER computing delta
        self.left_state.add(delta_a);
        self.right_state.add(delta_b);

        // Sum the three terms
        let mut result = term1;
        result.add(&term2);
        result.add(&term3);
        result
    }

    fn arity(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.left_state.clear();
        self.right_state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;
    use crate::circuit::store::Change;
    use crate::types::Path;
    use serde_json::json;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    fn setup_store() -> Store {
        let mut store = Store::new();
        store.ensure_collection("users");
        store.ensure_collection("posts");
        store.apply_change(&Change::create(
            "users",
            "user:1",
            json!({"id": "user:1", "name": "alice"}),
        ));
        store.apply_change(&Change::create(
            "posts",
            "post:1",
            json!({"id": "post:1", "author": "user:1"}),
        ));
        store
    }

    #[test]
    fn snapshot_produces_matching_pairs() {
        let store = setup_store();
        let join = Join::new(JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("author"),
        });

        let left = zset(&[("users:1", 1)]);
        let right = zset(&[("posts:1", 1)]);
        let result = join.snapshot(&[&left, &right], &store, None);

        assert!(result.get("users:1").is_some());
    }

    #[test]
    fn step_handles_deletion_from_one_side() {
        let store = setup_store();
        let condition = JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("author"),
        };
        let mut join = Join::new(condition);

        // Initial step: add both sides
        let da = zset(&[("users:1", 1)]);
        let db = zset(&[("posts:1", 1)]);
        let _ = join.step(&[&da, &db], &store, None);

        // Delete the user → should produce negative weight in output
        let da2 = zset(&[("users:1", -1)]);
        let empty: ZSet = HashMap::new();
        let result = join.step(&[&da2, &empty], &store, None);

        assert!(result.get("users:1").map(|w| *w < 0).unwrap_or(false));
    }

    #[test]
    fn step_matches_snapshot_diff() {
        // The fundamental DBSP correctness property:
        // step(dA, dB) == snapshot(A+dA, B+dB) - snapshot(A, B)
        let mut store = setup_store();
        let condition = JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("author"),
        };

        // Initial state
        let state_a = zset(&[("users:1", 1)]);
        let state_b = zset(&[("posts:1", 1)]);
        let snap_before =
            Join::new(condition.clone()).snapshot(&[&state_a, &state_b], &store, None);

        // Add user:2 and post:2
        store.apply_change(&Change::create(
            "users",
            "user:2",
            json!({"id": "user:2", "name": "bob"}),
        ));
        store.apply_change(&Change::create(
            "posts",
            "post:2",
            json!({"id": "post:2", "author": "user:2"}),
        ));

        let new_a = zset(&[("users:1", 1), ("users:2", 1)]);
        let new_b = zset(&[("posts:1", 1), ("posts:2", 1)]);
        let snap_after =
            Join::new(condition.clone()).snapshot(&[&new_a, &new_b], &store, None);

        let expected_delta = snap_before.diff(&snap_after);

        // Compute incrementally
        let delta_a = zset(&[("users:2", 1)]);
        let delta_b = zset(&[("posts:2", 1)]);
        let mut join = Join::new(condition);
        join.left_state = state_a;
        join.right_state = state_b;

        let actual_delta = join.step(&[&delta_a, &delta_b], &store, None);

        assert_eq!(actual_delta, expected_delta);
    }
}
