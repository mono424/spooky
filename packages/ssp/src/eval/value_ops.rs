use crate::types::{Path, SpookyValue};
use std::cmp::Ordering;

/// Resolve a nested value from a SpookyValue by following a Path.
pub fn resolve_field<'a>(value: Option<&'a SpookyValue>, path: &Path) -> Option<&'a SpookyValue> {
    let mut current = value?;
    for segment in path.segments() {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Compare two SpookyValues for ordering.
pub fn compare_values(a: Option<&SpookyValue>, b: Option<&SpookyValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => compare_values_inner(a, b),
    }
}

fn compare_values_inner(a: &SpookyValue, b: &SpookyValue) -> Ordering {
    match (a, b) {
        (SpookyValue::Null, SpookyValue::Null) => Ordering::Equal,
        (SpookyValue::Null, _) => Ordering::Less,
        (_, SpookyValue::Null) => Ordering::Greater,
        (SpookyValue::Number(a), SpookyValue::Number(b)) => {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (SpookyValue::Str(a), SpookyValue::Str(b)) => a.cmp(b),
        (SpookyValue::Bool(a), SpookyValue::Bool(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Hash a SpookyValue for use in join index lookups.
pub fn hash_value(value: &SpookyValue) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match value {
        SpookyValue::Null => 0u8.hash(&mut hasher),
        SpookyValue::Bool(b) => b.hash(&mut hasher),
        SpookyValue::Number(n) => n.to_bits().hash(&mut hasher),
        SpookyValue::Str(s) => s.hash(&mut hasher),
        SpookyValue::Array(_) => 2u8.hash(&mut hasher),
        SpookyValue::Object(_) => 3u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Normalize a record ID value (strip table prefix if present in a string).
pub fn normalize_record_id(value: SpookyValue) -> SpookyValue {
    if let SpookyValue::Str(s) = &value {
        if let Some((_table, id)) = s.split_once(':') {
            return SpookyValue::Str(id.to_string());
        }
    }
    value
}
