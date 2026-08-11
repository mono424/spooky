use crate::algebra::{ZSet, ZSetOps};
use crate::circuit::store::Store;
use crate::eval::value_ops::{compare_values, hash_value, resolve_field};
use crate::operator::plan::JoinCondition;
use crate::types::Sp00kyValue;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Anti-join operator: emits left rows whose join key has **no** match on the
/// right side. Output is keyed only by left-side keys with weights in `{0, 1}`.
///
/// Used by the policy rewriter when lowering `NOT IN (SELECT … WHERE …)` and
/// `NOT EXISTS (…)` permission subqueries: the outer scan becomes the left,
/// the inner subquery's plan becomes the right, and the correlation predicate
/// becomes the join condition.
///
/// Anti-joins are inherently non-incremental in the simple sense — a single
/// new right row can flip many left rows from "in" to "out". We use the same
/// `I → snapshot → D` pattern as `Distinct`: integrate, recompute the snapshot
/// from accumulated state, differentiate against the previous output. This is
/// O(|left|×|right|) per step worst case, which is acceptable at permission
/// scale.
#[derive(Debug)]
pub struct AntiJoin {
    pub condition: JoinCondition,
    /// Z⁻¹: accumulated left input state.
    left_state: ZSet,
    /// Z⁻¹: accumulated right input state.
    right_state: ZSet,
    /// Last emitted thresholded output, used for differentiation.
    prev_output: ZSet,
}

impl AntiJoin {
    pub fn new(condition: JoinCondition) -> Self {
        Self {
            condition,
            left_state: HashMap::new(),
            right_state: HashMap::new(),
            prev_output: HashMap::new(),
        }
    }

    /// Compute the anti-join snapshot: emit each left key with weight 1 iff
    /// **no** right row has the join key value present.
    fn anti_join(left: &ZSet, right: &ZSet, condition: &JoinCondition, store: &Store) -> ZSet {
        if left.is_empty() {
            return HashMap::new();
        }

        let mut right_present: HashMap<u64, Vec<Sp00kyValue>> = HashMap::new();
        for (r_key, &r_weight) in right {
            if r_weight <= 0 {
                continue;
            }
            if let Some(r_val) = store.get_row_by_key(r_key) {
                if let Some(r_field) = resolve_field(Some(r_val), &condition.right_field) {
                    let h = hash_value(r_field);
                    right_present.entry(h).or_default().push(r_field.clone());
                }
            }
        }

        let mut out = HashMap::new();
        for (l_key, &l_weight) in left {
            if l_weight <= 0 {
                continue;
            }
            let row = match store.get_row_by_key(l_key) {
                Some(r) => r,
                None => continue,
            };
            let l_field = match resolve_field(Some(row), &condition.left_field) {
                Some(v) => v,
                None => {
                    // Field absent on left row → no possible match → counts as
                    // "anti": include with weight 1.
                    out.insert(l_key.clone(), 1i64);
                    continue;
                }
            };
            let matched = right_present
                .get(&hash_value(l_field))
                .map(|cands| {
                    cands
                        .iter()
                        .any(|m| compare_values(Some(l_field), Some(m)) == Ordering::Equal)
                })
                .unwrap_or(false);
            if !matched {
                out.insert(l_key.clone(), 1i64);
            }
        }
        out
    }
}

impl super::Operator for AntiJoin {
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, _ctx: Option<&Sp00kyValue>) -> ZSet {
        Self::anti_join(inputs[0], inputs[1], &self.condition, store)
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> ZSet {
        self.left_state.add(input_deltas[0]);
        self.right_state.add(input_deltas[1]);

        let new_output = Self::anti_join(
            &self.left_state,
            &self.right_state,
            &self.condition,
            store,
        );
        let delta_out = self.prev_output.diff(&new_output);
        self.prev_output = new_output;
        delta_out
    }

    fn arity(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.left_state.clear();
        self.right_state.clear();
        self.prev_output.clear();
    }

    fn state_bytes(&self) -> usize {
        crate::size::zset_bytes(&self.left_state)
            + crate::size::zset_bytes(&self.right_state)
            + crate::size::zset_bytes(&self.prev_output)
    }

    fn evaluate_key(
        &self,
        key: &str,
        input_evals: &[bool],
        store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> bool {
        if !input_evals.first().copied().unwrap_or(false) {
            return false;
        }
        // Witness-check this row's join field against the integrated
        // right-side state (up to date: step() ran for every node before
        // the membership re-evaluation pass calls evaluate_key). Anti-join
        // admits the key iff NO witness exists; a missing left field counts
        // as anti, mirroring anti_join().
        let Some(l_val) = store.get_row_by_key(key) else {
            return false;
        };
        let Some(l_field) = resolve_field(Some(l_val), &self.condition.left_field) else {
            return true;
        };
        !self.right_state.iter().any(|(r_key, &w)| {
            w > 0
                && store
                    .get_row_by_key(r_key)
                    .and_then(|r_val| resolve_field(Some(r_val), &self.condition.right_field))
                    .map(|r_field| compare_values(Some(l_field), Some(r_field)) == Ordering::Equal)
                    .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::store::Change;
    use crate::operator::Operator;
    use crate::types::Path;
    use serde_json::json;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    fn setup_store() -> Store {
        let mut store = Store::new();
        store.ensure_collection("threads");
        store.ensure_collection("collab");
        store.apply_change(&Change::create(
            "threads",
            "thread:1",
            json!({"id": "thread:1"}),
        ));
        store.apply_change(&Change::create(
            "threads",
            "thread:2",
            json!({"id": "thread:2"}),
        ));
        store.apply_change(&Change::create(
            "collab",
            "collab:1",
            json!({"id": "collab:1", "out": "thread:1"}),
        ));
        store
    }

    fn cond() -> JoinCondition {
        JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("out"),
        }
    }

    #[test]
    fn snapshot_emits_left_keys_without_witnesses() {
        let store = setup_store();
        let aj = AntiJoin::new(cond());

        let left = zset(&[("threads:1", 1), ("threads:2", 1)]);
        let right = zset(&[("collab:1", 1)]);
        let result = aj.snapshot(&[&left, &right], &store, None);

        // thread:1 has a collaborator → excluded. thread:2 has none → included.
        assert!(!result.contains_key("threads:1"));
        assert_eq!(result.get("threads:2"), Some(&1));
    }

    #[test]
    fn snapshot_empty_right_includes_all_left() {
        let store = setup_store();
        let aj = AntiJoin::new(cond());

        let left = zset(&[("threads:1", 1), ("threads:2", 1)]);
        let right: ZSet = HashMap::new();
        let result = aj.snapshot(&[&left, &right], &store, None);

        assert_eq!(result.get("threads:1"), Some(&1));
        assert_eq!(result.get("threads:2"), Some(&1));
    }

    #[test]
    fn step_retracts_when_collaboration_arrives() {
        let store = setup_store();
        let mut aj = AntiJoin::new(cond());

        // Initial: both threads, no collaborations → both visible (anti).
        let dl = zset(&[("threads:1", 1), ("threads:2", 1)]);
        let dr_empty: ZSet = HashMap::new();
        let r0 = aj.step(&[&dl, &dr_empty], &store, None);
        assert_eq!(r0.get("threads:1"), Some(&1));
        assert_eq!(r0.get("threads:2"), Some(&1));

        // Add a collaborator on thread:1 → it should disappear from the anti set.
        let dl_empty: ZSet = HashMap::new();
        let dr1 = zset(&[("collab:1", 1)]);
        let r1 = aj.step(&[&dl_empty, &dr1], &store, None);
        assert_eq!(r1.get("threads:1"), Some(&-1));
        assert!(!r1.contains_key("threads:2"));
    }

    #[test]
    fn step_re_admits_when_collaboration_revoked() {
        let store = setup_store();
        let mut aj = AntiJoin::new(cond());

        // Prime: thread:1 has a collab → excluded. thread:2 visible.
        let dl = zset(&[("threads:1", 1), ("threads:2", 1)]);
        let dr1 = zset(&[("collab:1", 1)]);
        let _ = aj.step(&[&dl, &dr1], &store, None);

        // Revoke the collab → thread:1 should re-enter.
        let dl_empty: ZSet = HashMap::new();
        let dr2 = zset(&[("collab:1", -1)]);
        let r2 = aj.step(&[&dl_empty, &dr2], &store, None);
        assert_eq!(r2.get("threads:1"), Some(&1));
    }

    #[test]
    fn step_matches_snapshot_diff() {
        let mut store = setup_store();
        let condition = cond();

        let state_a = zset(&[("threads:1", 1), ("threads:2", 1)]);
        let state_b = zset(&[("collab:1", 1)]);
        let snap_before =
            AntiJoin::new(condition.clone()).snapshot(&[&state_a, &state_b], &store, None);

        store.apply_change(&Change::create(
            "threads",
            "thread:3",
            json!({"id": "thread:3"}),
        ));
        store.apply_change(&Change::create(
            "collab",
            "collab:2",
            json!({"id": "collab:2", "out": "thread:2"}),
        ));

        let new_a = zset(&[("threads:1", 1), ("threads:2", 1), ("threads:3", 1)]);
        let new_b = zset(&[("collab:1", 1), ("collab:2", 1)]);
        let snap_after =
            AntiJoin::new(condition.clone()).snapshot(&[&new_a, &new_b], &store, None);
        let expected_delta = snap_before.diff(&snap_after);

        let mut aj = AntiJoin::new(condition);
        aj.left_state = state_a;
        aj.right_state = state_b;
        aj.prev_output = snap_before;

        let dl = zset(&[("threads:3", 1)]);
        let dr = zset(&[("collab:2", 1)]);
        let actual_delta = aj.step(&[&dl, &dr], &store, None);

        assert_eq!(actual_delta, expected_delta);
    }
}
