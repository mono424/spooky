use crate::engine::circuit::Database;
use crate::engine::types::{FastMap, Path, SpookyValue, ZSet};
use rustc_hash::FxHasher;
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::hash::Hasher;

/// Numeric comparison operation
#[derive(Clone, Copy)]
pub enum NumericOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
}

/// Resolve nested value using dot notation path
#[inline(always)]
pub fn resolve_nested_value<'a>(
    root: Option<&'a SpookyValue>,
    path: &Path,
) -> Option<&'a SpookyValue> {
    let mut current = root;
    for part in &path.0 {
        match current {
            Some(SpookyValue::Object(map)) => {
                current = map.get(part);
            }
            _ => return None,
        }
    }
    current
}

/// Compare two SpookyValues for ordering
pub fn compare_spooky_values(a: Option<&SpookyValue>, b: Option<&SpookyValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (SpookyValue::Null, SpookyValue::Null) => Ordering::Equal,
            (SpookyValue::Bool(ba), SpookyValue::Bool(bb)) => ba.cmp(bb),
            (SpookyValue::Number(na), SpookyValue::Number(nb)) => {
                na.partial_cmp(nb).unwrap_or(Ordering::Equal)
            }
            (SpookyValue::Str(sa), SpookyValue::Str(sb)) => sa.cmp(sb),
            (SpookyValue::Array(aa), SpookyValue::Array(ab)) => {
                let len_cmp = aa.len().cmp(&ab.len());
                if len_cmp != Ordering::Equal {
                    return len_cmp;
                }
                for (ia, ib) in aa.iter().zip(ab.iter()) {
                    let cmp = compare_spooky_values(Some(ia), Some(ib));
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                }
                Ordering::Equal
            }
            (SpookyValue::Object(oa), SpookyValue::Object(ob)) => oa.len().cmp(&ob.len()),
            _ => type_rank(va).cmp(&type_rank(vb)),
        },
    }
}

fn type_rank(v: &SpookyValue) -> u8 {
    match v {
        SpookyValue::Null => 0,
        SpookyValue::Bool(_) => 1,
        SpookyValue::Number(_) => 2,
        SpookyValue::Str(_) => 3,
        SpookyValue::Array(_) => 4,
        SpookyValue::Object(_) => 5,
    }
}

/// Fast hashing for join keys
#[inline(always)]
pub fn hash_spooky_value(v: &SpookyValue) -> u64 {
    let mut hasher = FxHasher::default();
    hash_value_recursive(v, &mut hasher);
    hasher.finish()
}

fn hash_value_recursive(v: &SpookyValue, hasher: &mut FxHasher) {
    match v {
        SpookyValue::Null => hasher.write_u8(0),
        SpookyValue::Bool(b) => {
            hasher.write_u8(1);
            hasher.write_u8(*b as u8);
        }
        SpookyValue::Number(n) => {
            hasher.write_u8(2);
            hasher.write_u64(n.to_bits());
        }
        SpookyValue::Str(s) => {
            hasher.write_u8(3);
            hasher.write(s.as_bytes());
        }
        SpookyValue::Array(arr) => {
            hasher.write_u8(4);
            for item in arr {
                hash_value_recursive(item, hasher);
            }
        }
        SpookyValue::Object(obj) => {
            hasher.write_u8(5);
            for (k, v) in obj {
                hasher.write(k.as_bytes());
                hash_value_recursive(v, hasher);
            }
        }
    }
}

/// Extract a column of f64 values from ZSet for SIMD processing
#[inline(always)]
pub fn extract_number_column(
    zset: &ZSet,
    path: &Path,
    db: &Database,
) -> (Vec<SmolStr>, Vec<i64>, Vec<f64>) {
    let mut ids = Vec::with_capacity(zset.len());
    let mut weights = Vec::with_capacity(zset.len());
    let mut numbers = Vec::with_capacity(zset.len());

    for (key, weight) in zset {
        let val_opt = if let Some((table, _)) = key.split_once(':') {
            db.tables.get(table).and_then(|t| t.rows.get(key))
        } else {
            None
        };

        let num_val = if let Some(row_val) = val_opt {
            if let Some(SpookyValue::Number(n)) = resolve_nested_value(Some(row_val), path) {
                *n
            } else {
                f64::NAN
            }
        } else {
            f64::NAN
        };

        ids.push(key.clone());
        weights.push(*weight);
        numbers.push(num_val);
    }

    (ids, weights, numbers)
}

/// Auto-vectorizable batch filter for f64 values
pub fn filter_f64_batch(values: &[f64], target: f64, op: NumericOp) -> Vec<usize> {
    let mut indices = Vec::with_capacity(values.len());
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();

    let mut i = 0;
    for chunk in chunks {
        for val in chunk {
            let pass = match op {
                NumericOp::Gt => *val > target,
                NumericOp::Gte => *val >= target,
                NumericOp::Lt => *val < target,
                NumericOp::Lte => *val <= target,
                NumericOp::Eq => (*val - target).abs() < f64::EPSILON,
                NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
            };
            if pass {
                indices.push(i);
            }
            i += 1;
        }
    }

    for val in remainder {
        let pass = match op {
            NumericOp::Gt => *val > target,
            NumericOp::Gte => *val >= target,
            NumericOp::Lt => *val < target,
            NumericOp::Lte => *val <= target,
            NumericOp::Eq => (*val - target).abs() < f64::EPSILON,
            NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
        };
        if pass {
            indices.push(i);
        }
        i += 1;
    }

    indices
}

/// Portable SIMD sum (for future aggregations)
#[allow(dead_code)]
#[inline(always)]
pub fn sum_f64_simd(values: &[f64]) -> f64 {
    let mut sums = [0.0; 8];
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks {
        for i in 0..8 {
            sums[i] += chunk[i];
        }
    }

    let mut total: f64 = sums.iter().sum();
    for v in remainder {
        total += v;
    }
    total
}

/// Normalize RecordId-like objects to string format
pub fn normalize_record_id(val: SpookyValue) -> SpookyValue {
    if let SpookyValue::Object(map) = &val {
        let table_key = SmolStr::new("tb");
        let table_key_alt = SmolStr::new("table");
        let id_key = SmolStr::new("id");

        if let (Some(table_val), Some(id_val)) = (
            map.get(&table_key).or_else(|| map.get(&table_key_alt)),
            map.get(&id_key),
        ) {
            let table_str = match table_val {
                SpookyValue::Str(s) => s.to_string(),
                SpookyValue::Number(n) => n.to_string(),
                _ => return val,
            };
            let id_str = match id_val {
                SpookyValue::Str(s) => s.to_string(),
                SpookyValue::Number(n) => n.to_string(),
                _ => return val,
            };
            return SpookyValue::Str(SmolStr::new(format!("{}:{}", table_str, id_str)));
        }
    }
    val
}

// --- SIMD FILTER CONSOLIDATION ---

use crate::engine::operators::Predicate;
use serde_json::Value;

/// Configuration for SIMD-friendly numeric filtering.
/// Holds the extracted field path, target value, and comparison operation.
pub struct NumericFilterConfig<'a> {
    pub path: &'a Path,
    pub target: f64,
    pub op: NumericOp,
}

impl<'a> NumericFilterConfig<'a> {
    /// Try to extract numeric filter config from a predicate.
    /// Returns None if the predicate is not a simple numeric comparison.
    pub fn from_predicate(pred: &'a Predicate) -> Option<Self> {
        // Extract target value and operation
        let (target, op) = match pred {
            Predicate::Gt {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Gt),
            Predicate::Gte {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Gte),
            Predicate::Lt {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Lt),
            Predicate::Lte {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Lte),
            Predicate::Eq {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Eq),
            Predicate::Neq {
                value: Value::Number(n),
                ..
            } => (n.as_f64()?, NumericOp::Neq),
            _ => return None,
        };

        // Extract field path
        let path = match pred {
            Predicate::Gt { field, .. }
            | Predicate::Gte { field, .. }
            | Predicate::Lt { field, .. }
            | Predicate::Lte { field, .. }
            | Predicate::Eq { field, .. }
            | Predicate::Neq { field, .. } => field,
            _ => return None,
        };

        Some(Self { path, target, op })
    }
}

/// Apply SIMD-optimized numeric filter to a ZSet.
/// Uses extract_number_column and filter_f64_batch for vectorized filtering.
#[inline]
pub fn apply_numeric_filter(upstream: &ZSet, config: &NumericFilterConfig, db: &Database) -> ZSet {
    let (keys, weights, numbers) = extract_number_column(upstream, config.path, db);
    let passing_indices = filter_f64_batch(&numbers, config.target, config.op);

    let mut out = FastMap::default();
    for idx in passing_indices {
        out.insert(keys[idx].clone(), weights[idx]);
    }
    out
}
