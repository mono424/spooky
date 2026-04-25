use crate::types::{Path, Sp00kyValue};
use std::cmp::Ordering;

/// Resolve a nested value from a Sp00kyValue by following a Path.
pub fn resolve_field<'a>(value: Option<&'a Sp00kyValue>, path: &Path) -> Option<&'a Sp00kyValue> {
    let mut current = value?;
    for segment in path.segments() {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Compare two Sp00kyValues for ordering.
pub fn compare_values(a: Option<&Sp00kyValue>, b: Option<&Sp00kyValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => compare_values_inner(a, b),
    }
}

fn compare_values_inner(a: &Sp00kyValue, b: &Sp00kyValue) -> Ordering {
    match (a, b) {
        (Sp00kyValue::Null, Sp00kyValue::Null) => Ordering::Equal,
        (Sp00kyValue::Null, _) => Ordering::Less,
        (_, Sp00kyValue::Null) => Ordering::Greater,
        (Sp00kyValue::Int(a), Sp00kyValue::Int(b)) => a.cmp(b),
        (Sp00kyValue::Float(a), Sp00kyValue::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Sp00kyValue::Int(a), Sp00kyValue::Float(b)) => (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal),
        (Sp00kyValue::Float(a), Sp00kyValue::Int(b)) => a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal),
        (Sp00kyValue::Str(a), Sp00kyValue::Str(b)) => a.cmp(b),
        (Sp00kyValue::Bool(a), Sp00kyValue::Bool(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Hash a Sp00kyValue for use in join index lookups.
pub fn hash_value(value: &Sp00kyValue) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match value {
        Sp00kyValue::Null => 0u8.hash(&mut hasher),
        Sp00kyValue::Bool(b) => b.hash(&mut hasher),
        // Cast Int through f64 so Int(5) and Float(5.0) hash equally — join
        // index lookups need to match across numeric types.
        Sp00kyValue::Int(n) => (*n as f64).to_bits().hash(&mut hasher),
        Sp00kyValue::Float(n) => n.to_bits().hash(&mut hasher),
        Sp00kyValue::Str(s) => s.hash(&mut hasher),
        Sp00kyValue::Array(_) => 2u8.hash(&mut hasher),
        Sp00kyValue::Object(_) => 3u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Normalize a record ID value (strip table prefix if present in a string).
pub fn normalize_record_id(value: Sp00kyValue) -> Sp00kyValue {
    if let Sp00kyValue::Str(s) = &value {
        if let Some((_table, id)) = s.split_once(':') {
            return Sp00kyValue::Str(id.to_string());
        }
    }
    value
}
