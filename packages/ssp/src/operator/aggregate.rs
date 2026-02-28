use crate::algebra::{ZSet, ZSetOps};
use crate::circuit::store::Store;
use crate::eval::value_ops::resolve_field;
use crate::types::{Path, SpookyValue};
use std::collections::HashMap;

/// Supported aggregate functions.
#[derive(Clone, Debug)]
pub enum AggregateFunc {
    Count,
    Sum { field: Path },
}

/// Per-group aggregate state.
#[derive(Debug, Clone, Default)]
struct AggState {
    count: i64,
    sums: Vec<f64>,
}

/// Aggregate operator with per-group running state (Z⁻¹).
///
/// For each input delta record with weight w:
///   - COUNT: running_count += w
///   - SUM(field): running_sum += field_value * w
///
/// Emits a delta reflecting the change in aggregate output.
#[derive(Debug)]
pub struct Aggregate {
    pub group_by: Option<Vec<Path>>,
    pub funcs: Vec<AggregateFunc>,
    /// Per-group accumulated state.
    group_state: HashMap<String, AggState>,
    /// Previous output (for computing output delta).
    prev_output: ZSet,
}

impl Aggregate {
    pub fn new(group_by: Option<Vec<Path>>, funcs: Vec<AggregateFunc>) -> Self {
        Self {
            group_by,
            funcs,
            group_state: HashMap::new(),
            prev_output: HashMap::new(),
        }
    }

    /// Compute a group key for a record.
    fn group_key(&self, key: &str, store: &Store) -> String {
        match &self.group_by {
            None => "__global__".to_string(),
            Some(fields) => {
                let row = store.get_row_by_key(key);
                let parts: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        row.and_then(|r| resolve_field(Some(r), f))
                            .map(|v| format!("{:?}", v))
                            .unwrap_or_else(|| "null".to_string())
                    })
                    .collect();
                parts.join("|")
            }
        }
    }

    /// Build the output Z-set from current group state.
    fn build_output(&self) -> ZSet {
        let mut out = HashMap::new();
        for (group_key, state) in &self.group_state {
            if state.count > 0 {
                out.insert(group_key.clone(), 1);
            }
        }
        out
    }
}

impl super::Operator for Aggregate {
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, _ctx: Option<&SpookyValue>) -> ZSet {
        let upstream = inputs[0];
        let mut groups: HashMap<String, AggState> = HashMap::new();

        for (key, &weight) in upstream {
            if weight <= 0 {
                continue;
            }
            let gk = self.group_key(key, store);
            let state = groups.entry(gk).or_default();
            state.count += weight;

            let row = store.get_row_by_key(key);
            for (i, func) in self.funcs.iter().enumerate() {
                if let AggregateFunc::Sum { field } = func {
                    let val = row
                        .and_then(|r| resolve_field(Some(r), field))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    if state.sums.len() <= i {
                        state.sums.resize(i + 1, 0.0);
                    }
                    state.sums[i] += val * weight as f64;
                }
            }
        }

        let mut out = HashMap::new();
        for (group_key, state) in &groups {
            if state.count > 0 {
                out.insert(group_key.clone(), 1);
            }
        }
        out
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        _ctx: Option<&SpookyValue>,
    ) -> ZSet {
        let upstream_delta = input_deltas[0];

        for (key, &weight) in upstream_delta {
            let gk = self.group_key(key, store);
            let state = self.group_state.entry(gk).or_default();
            state.count += weight;

            let row = store.get_row_by_key(key);
            for (i, func) in self.funcs.iter().enumerate() {
                if let AggregateFunc::Sum { field } = func {
                    let val = row
                        .and_then(|r| resolve_field(Some(r), field))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    if state.sums.len() <= i {
                        state.sums.resize(i + 1, 0.0);
                    }
                    state.sums[i] += val * weight as f64;
                }
            }
        }

        let new_output = self.build_output();
        let delta_out = self.prev_output.diff(&new_output);
        self.prev_output = new_output;
        delta_out
    }

    fn arity(&self) -> usize {
        1
    }

    fn reset(&mut self) {
        self.group_state.clear();
        self.prev_output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;
    use crate::circuit::store::Change;
    use serde_json::json;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn count_increments_on_insert() {
        let store = Store::new();
        let mut agg = Aggregate::new(None, vec![AggregateFunc::Count]);

        let d1 = zset(&[("a", 1), ("b", 1)]);
        let result = agg.step(&[&d1], &store, None);
        assert!(!result.is_empty());
    }

    #[test]
    fn count_decrements_on_delete() {
        let store = Store::new();
        let mut agg = Aggregate::new(None, vec![AggregateFunc::Count]);

        let d1 = zset(&[("a", 1), ("b", 1)]);
        let _ = agg.step(&[&d1], &store, None);

        // Count went from 2 to 1 — group still exists, no membership delta
        // But if count goes to 0, group disappears
        let d2 = zset(&[("a", -1), ("b", -1)]);
        let result = agg.step(&[&d2], &store, None);
        // Group disappears → delta should show removal
        assert_eq!(result.get("__global__"), Some(&-1));
    }

    #[test]
    fn sum_weighted_delta() {
        let mut store = Store::new();
        store.ensure_collection("items");
        store.apply_change(&Change::create("items", "item:1", json!({"price": 100})));
        store.apply_change(&Change::create("items", "item:2", json!({"price": 200})));

        let mut agg = Aggregate::new(
            None,
            vec![AggregateFunc::Sum {
                field: Path::new("price"),
            }],
        );

        let d1 = zset(&[("items:1", 1), ("items:2", 1)]);
        let result = agg.step(&[&d1], &store, None);
        // Group appears
        assert!(!result.is_empty());

        // Delete item:1 → group still exists (count=1)
        let d2 = zset(&[("items:1", -1)]);
        let result2 = agg.step(&[&d2], &store, None);
        // Group still present, no membership delta
        assert!(result2.is_empty());
    }
}
