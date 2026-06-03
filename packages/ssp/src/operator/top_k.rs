use crate::algebra::ZSet;
use crate::circuit::store::Store;
use crate::eval::value_ops::resolve_field;
use crate::operator::plan::OrderSpec;
use crate::types::Sp00kyValue;
use std::collections::{BTreeSet, HashMap};

/// TopK operator with sorted buffer state (Z⁻¹).
///
/// Maintains a sorted buffer of all input records and emits the window
/// `[offset, offset + limit)` of it (SurrealQL `LIMIT limit START offset`).
/// On each delta:
///   1. Insert/remove records from the buffer
///   2. Compute which records enter/leave the window
///   3. Emit +1 for new entrants, -1 for displaced records
#[derive(Debug)]
pub struct TopK {
    pub limit: usize,
    /// Number of leading sorted rows to skip (SurrealQL `START`). 0 = top-N.
    pub offset: usize,
    pub order_by: Option<Vec<OrderSpec>>,
    /// All records seen so far, sorted. Each entry is (sort_key_parts, row_key).
    /// Using BTreeSet for automatic sorted order.
    buffer: BTreeSet<(Vec<SortableValue>, String)>,
    /// Reverse index: row_key → sort key parts (for removal)
    key_index: HashMap<String, Vec<SortableValue>>,
}

/// One field's sort key: an orderable scalar plus the field's direction.
///
/// `Ord` folds the direction in, so a `Vec<SortableValue>` compares
/// lexicographically across fields with mixed directions — which is what a
/// multi-key `ORDER BY a ASC, b DESC` needs. Crucially this works for *every*
/// scalar type, including strings: descending order is the reverse of the
/// scalar comparison, not a negated value. (The previous version negated
/// `Int`/`Bool` but left strings untouched with a "rely on reverse iteration"
/// note — but `current_window`/`snapshot` never reverse, so `DESC` on a string
/// or `datetime` field silently sorted ascending.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortableValue {
    scalar: Scalar,
    descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Scalar {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
}

impl PartialOrd for SortableValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortableValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let base = self.scalar.cmp(&other.scalar);
        // Within a Vec the same position always carries the same direction (it
        // comes from the same OrderSpec), so reversing on `self.descending` is
        // well-defined.
        if self.descending {
            base.reverse()
        } else {
            base
        }
    }
}

impl SortableValue {
    fn from_sp00ky(val: Option<&Sp00kyValue>, descending: bool) -> Self {
        let scalar = match val {
            None | Some(Sp00kyValue::Null) => Scalar::Null,
            Some(Sp00kyValue::Bool(b)) => Scalar::Bool(*b),
            Some(v) if v.as_f64().is_some() => {
                // Use integer representation for consistent ordering
                Scalar::Int((v.as_f64().unwrap() * 1_000_000.0) as i64)
            }
            Some(Sp00kyValue::Str(s)) => Scalar::Str(s.clone()),
            _ => Scalar::Null,
        };
        SortableValue { scalar, descending }
    }
}

impl TopK {
    pub fn new(limit: usize, offset: usize, order_by: Option<Vec<OrderSpec>>) -> Self {
        Self {
            limit,
            offset,
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
            None => vec![SortableValue {
                scalar: Scalar::Str(key.to_string()),
                descending: false,
            }],
        }
    }

    /// The keys currently in the output window `[offset, offset + limit)`.
    fn current_window(&self) -> Vec<String> {
        self.buffer
            .iter()
            .skip(self.offset)
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
        for (_, key) in items.into_iter().skip(self.offset).take(self.limit) {
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
        let old_top_k = self.current_window();

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

        let new_top_k = self.current_window();

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

    fn evaluate_key(
        &self,
        _key: &str,
        input_evals: &[bool],
        _store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> bool {
        // TopK can drop keys that fall outside the limit, but that
        // path is handled by the in-cache check at apply-delta time.
        // For the purpose of detecting Update-driven membership
        // transitions, treating it as a pass-through is correct
        // enough: a row newly admitted upstream may not actually end
        // up in the top-N, but the over-emit gets dedup'd by
        // `view.cache.contains_key` when classifying additions.
        input_evals.first().copied().unwrap_or(false)
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
            0,
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
    fn snapshot_offset_skips_leading_rows() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 40})));
        store.apply_change(&Change::create("posts", "post:3", json!({"score": 30})));
        store.apply_change(&Change::create("posts", "post:4", json!({"score": 20})));

        // ORDER BY score DESC → [40, 30, 20, 10]. LIMIT 2 START 1 → [30, 20].
        let top_k = TopK::new(
            2,
            1,
            Some(vec![OrderSpec {
                field: Path::new("score"),
                direction: "DESC".into(),
            }]),
        );
        let input = zset(&[("posts:1", 1), ("posts:2", 1), ("posts:3", 1), ("posts:4", 1)]);
        let result = top_k.snapshot(&[&input], &store, None);

        assert_eq!(result.len(), 2);
        assert!(result.is_present("posts:3")); // score 30 (skipped post:2 @ 40)
        assert!(result.is_present("posts:4")); // score 20
        assert!(!result.is_present("posts:2")); // skipped by START 1
        assert!(!result.is_present("posts:1")); // below the window
    }

    #[test]
    fn step_offset_emits_window_shift_on_insert() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 40})));
        store.apply_change(&Change::create("posts", "post:3", json!({"score": 20})));

        // LIMIT 1 START 1 → window is the 2nd row. Order [40, 20, 10] → [20].
        let mut top_k = TopK::new(
            1,
            1,
            Some(vec![OrderSpec {
                field: Path::new("score"),
                direction: "DESC".into(),
            }]),
        );
        let d1 = zset(&[("posts:1", 1), ("posts:2", 1), ("posts:3", 1)]);
        let _ = top_k.step(&[&d1], &store, None);

        // Insert score 30 → order [40, 30, 20, 10], window (idx 1) is now 30.
        store.apply_change(&Change::create("posts", "post:4", json!({"score": 30})));
        let d2 = zset(&[("posts:4", 1)]);
        let result = top_k.step(&[&d2], &store, None);

        assert_eq!(result.get("posts:4"), Some(&1)); // enters the window
        assert_eq!(result.get("posts:3"), Some(&-1)); // pushed out (now idx 2)
    }

    #[test]
    fn step_emits_displacement_on_insert() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 30})));

        let mut top_k = TopK::new(
            2,
            0,
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

    // Reproduces the solid-app game-list query window:
    //   ORDER BY sort_index ASC, date DESC LIMIT n START m
    // All rows share sort_index = 0 (the pre-migration default / a tie), so the
    // secondary `date DESC` is the deciding key — newest date must come first.
    // `date` is a SurrealDB `datetime`, which reaches the SSP as a `Str` (ISO
    // 8601). The window pages must be complete, non-overlapping, and ordered
    // newest→oldest.
    fn game(id: &str, date: &str) -> Change {
        Change::create("game", id, json!({ "sort_index": 0, "date": date }))
    }

    fn game_order() -> Option<Vec<OrderSpec>> {
        Some(vec![
            OrderSpec { field: Path::new("sort_index"), direction: "ASC".into() },
            OrderSpec { field: Path::new("date"), direction: "DESC".into() },
        ])
    }

    #[test]
    fn paged_window_orders_datetime_string_descending() {
        let mut store = Store::new();
        store.ensure_collection("game");
        store.apply_change(&game("game:a", "2020-01-01T00:00:00Z"));
        store.apply_change(&game("game:b", "2021-01-01T00:00:00Z"));
        store.apply_change(&game("game:c", "2022-01-01T00:00:00Z"));
        store.apply_change(&game("game:d", "2023-01-01T00:00:00Z"));
        store.apply_change(&game("game:e", "2024-01-01T00:00:00Z"));
        let input = zset(&[
            ("game:a", 1), ("game:b", 1), ("game:c", 1), ("game:d", 1), ("game:e", 1),
        ]);

        // Expected global order (sort_index ASC tie → date DESC): e, d, c, b, a.
        // Page 0 = LIMIT 2 START 0 → [e, d] (newest two).
        let p0 = TopK::new(2, 0, game_order()).snapshot(&[&input], &store, None);
        assert_eq!(p0.len(), 2);
        assert!(p0.is_present("game:e"), "page 0 must hold the newest game");
        assert!(p0.is_present("game:d"), "page 0 must hold the 2nd-newest game");

        // Page 1 = LIMIT 2 START 2 → [c, b].
        let p1 = TopK::new(2, 2, game_order()).snapshot(&[&input], &store, None);
        assert_eq!(p1.len(), 2, "page 1 (START 2) must be a full window");
        assert!(p1.is_present("game:c"));
        assert!(p1.is_present("game:b"));

        // Page 2 = LIMIT 2 START 4 → [a] (short tail = real end).
        let p2 = TopK::new(2, 4, game_order()).snapshot(&[&input], &store, None);
        assert_eq!(p2.len(), 1);
        assert!(p2.is_present("game:a"));

        // Windows must not overlap.
        assert!(!p0.is_present("game:c") && !p0.is_present("game:a"));
        assert!(!p1.is_present("game:e") && !p1.is_present("game:a"));
    }

    #[test]
    fn step_no_displacement_when_below_cutoff() {
        let mut store = Store::new();
        store.ensure_collection("posts");
        store.apply_change(&Change::create("posts", "post:1", json!({"score": 10})));
        store.apply_change(&Change::create("posts", "post:2", json!({"score": 30})));

        let mut top_k = TopK::new(
            2,
            0,
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
