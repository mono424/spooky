use crate::algebra::ZSet;
use crate::circuit::store::Store;
use crate::eval::value_ops::resolve_field;
use crate::operator::plan::OrderSpec;
use crate::types::Sp00kyValue;
use std::collections::{BTreeSet, HashMap};

/// TopK operator with sorted buffer state (Z⁻¹).
///
/// Maintains a sorted buffer of all input records. On each delta:
///   1. Insert/remove records from the buffer
///   2. Compute which records enter/leave the top K
///   3. Emit +1 for new entrants, -1 for displaced records
#[derive(Debug)]
pub struct TopK {
    pub limit: usize,
    pub order_by: Option<Vec<OrderSpec>>,
    /// All records seen so far, sorted. Each entry is (sort_key_parts, row_key).
    /// Using BTreeSet for automatic sorted order.
    buffer: BTreeSet<(Vec<SortableValue>, String)>,
    /// Reverse index: row_key → sort key parts (for removal)
    key_index: HashMap<String, Vec<SortableValue>>,
}

/// A value that implements Ord for sorting purposes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortableValue {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
}

impl SortableValue {
    fn from_sp00ky(val: Option<&Sp00kyValue>, descending: bool) -> Self {
        let sv = match val {
            None | Some(Sp00kyValue::Null) => SortableValue::Null,
            Some(Sp00kyValue::Bool(b)) => SortableValue::Bool(*b),
            Some(v) if v.as_f64().is_some() => {
                // Use integer representation for consistent ordering
                SortableValue::Int((v.as_f64().unwrap() * 1_000_000.0) as i64)
            }
            Some(Sp00kyValue::Str(s)) => SortableValue::Str(s.clone()),
            _ => SortableValue::Null,
        };
        if descending {
            // For descending, negate the sort key
            match sv {
                SortableValue::Int(n) => SortableValue::Int(-n),
                SortableValue::Bool(b) => SortableValue::Bool(!b),
                other => other, // Strings: rely on reverse iteration
            }
        } else {
            sv
        }
    }
}

impl TopK {
    pub fn new(limit: usize, order_by: Option<Vec<OrderSpec>>) -> Self {
        Self {
            limit,
            order_by,
            buffer: BTreeSet::new(),
            key_index: HashMap::new(),
        }
    }

    fn compute_sort_key(&self, key: &str, store: &Store) -> Vec<SortableValue> {
        let row = store.get_row_by_key(key);
        match &self.order_by {
            Some(orders) => orders
                .iter()
                .map(|ord| {
                    let val = row.and_then(|r| resolve_field(Some(r), &ord.field));
                    let desc = ord.direction.eq_ignore_ascii_case("DESC");
                    SortableValue::from_sp00ky(val, desc)
                })
                .collect(),
            None => vec![SortableValue::Str(key.to_string())],
        }
    }

    fn current_top_k(&self) -> Vec<String> {
        self.buffer
            .iter()
            .take(self.limit)
            .map(|(_, key)| key.clone())
            .collect()
    }
}

impl super::Operator for TopK {
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, _ctx: Option<&Sp00kyValue>) -> ZSet {
        let upstream = inputs[0];
        let mut items: Vec<(Vec<SortableValue>, &String)> = upstream
            .iter()
            .filter(|(_, &w)| w > 0)
            .map(|(key, _)| (self.compute_sort_key(key, store), key))
            .collect();

        items.sort();

        let mut out = HashMap::new();
        for (_, key) in items.into_iter().take(self.limit) {
            out.insert(key.clone(), 1);
        }
        out
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> ZSet {
        let upstream_delta = input_deltas[0];
        let old_top_k = self.current_top_k();

        for (key, &weight) in upstream_delta {
            if weight > 0 {
                let sort_key = self.compute_sort_key(key, store);
                self.buffer.insert((sort_key.clone(), key.clone()));
                self.key_index.insert(key.clone(), sort_key);
            } else if weight < 0 {
                if let Some(sort_key) = self.key_index.remove(key) {
                    self.buffer.remove(&(sort_key, key.clone()));
                }
            }
        }

        let new_top_k = self.current_top_k();

        // Compute displacement delta
        let mut output_delta = HashMap::new();
        let old_set: std::collections::HashSet<&String> = old_top_k.iter().collect();
        let new_set: std::collections::HashSet<&String> = new_top_k.iter().collect();

        for key in &new_top_k {
            if !old_set.contains(key) {
                *output_delta.entry(key.clone()).or_insert(0) += 1;
            }
        }
        for key in &old_top_k {
            if !new_set.contains(key) {
                *output_delta.entry(key.clone()).or_insert(0) -= 1;
            }
        }

        output_delta.retain(|_, w| *w != 0);
        output_delta
    }

    fn arity(&self) -> usize {
        1
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.key_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;
    use crate::algebra::ZSetOps;
    use crate::circuit::store::Change;
    use crate::types::Path;
    use serde_json::json;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn snapshot_returns_top_k_entries() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 30})));
        store.apply_change(&Change::create("posts", "post:3", json!({"score": 20})));

        let top_k = TopK::new(
            2,
            Some(vec![OrderSpec {
                field: Path::new("score"),
                direction: "DESC".into(),
            }]),
        );
        let input = zset(&[("posts:1", 1), ("posts:2", 1), ("posts:3", 1)]);
        let result = top_k.snapshot(&[&input], &store, None);

        assert_eq!(result.len(), 2);
        assert!(result.is_present("posts:2")); // score 30
        assert!(result.is_present("posts:3")); // score 20
    }

    #[test]
    fn step_emits_displacement_on_insert() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 30})));

        let mut top_k = TopK::new(
            2,
            Some(vec![OrderSpec {
                field: Path::new("score"),
                direction: "DESC".into(),
            }]),
        );

        // Initial: top 2 = [post:2(30), post:1(10)]
        let d1 = zset(&[("posts:1", 1), ("posts:2", 1)]);
        let _ = top_k.step(&[&d1], &store, None);

        // Insert post:3 with score 20 → displaces post:1
        store.apply_change(&Change::create("posts", "post:3", json!({"score": 20})));
        let d2 = zset(&[("posts:3", 1)]);
        let result = top_k.step(&[&d2], &store, None);

        assert_eq!(result.get("posts:3"), Some(&1)); // enters top-K
        assert_eq!(result.get("posts:1"), Some(&-1)); // displaced
    }

    #[test]
    fn step_no_displacement_when_below_cutoff() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 30})));

        let mut top_k = TopK::new(
            2,
            Some(vec![OrderSpec {
                field: Path::new("score"),
                direction: "DESC".into(),
            }]),
        );
        let d1 = zset(&[("posts:1", 1), ("posts:2", 1)]);
        let _ = top_k.step(&[&d1], &store, None);

        // Insert post:3 with score 5 → below cutoff
        store.apply_change(&Change::create("posts", "post:3", json!({"score": 5})));
        let d2 = zset(&[("posts:3", 1)]);
        let result = top_k.step(&[&d2], &store, None);

        assert!(result.is_empty());
    }
}
