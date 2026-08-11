use crate::eval::value_ref::ValueRef;
use crate::types::{Path, Sp00kyValue};
use std::cmp::Ordering;

/// Resolve a nested value by following a Path.
///
/// Returns [`ValueRef::Missing`] when the path does not resolve, which folds
/// away the `Option` the caller used to unwrap.
///
/// One non-obvious case: `<field>.id` on a record-reference field. The
/// store keeps record references as the string form `"table:abc"`, but
/// SurrealDB's predicate semantics auto-dereference `author.id` on a
/// `record<user>` field to the linked record's id. So when we land on a
/// string of the form `"<table>:<rest>"` and the next segment is `id`,
/// return the string itself — that *is* the id. Without this, every
/// permission rule of the shape `author.id = $auth.id` (the example
/// app's thread permission) silently evaluates to false on records the
/// caller actually owns.
pub fn resolve_field<'a>(value: ValueRef<'a>, path: &Path) -> ValueRef<'a> {
    let mut current = value;
    if current.is_missing() {
        return ValueRef::Missing;
    }
    for segment in path.segments() {
        if segment == "id" && current.is_record_ref() {
            return current;
        }
        current = current.get(segment);
        if current.is_missing() {
            return ValueRef::Missing;
        }
    }
    current
}

/// Compare two values for ordering.
///
/// Ordering is unchanged from the `Option<&Sp00kyValue>` version this
/// replaces: missing sorts below null, null below every other value, and
/// **values of different types compare Equal**. That last rule is odd but
/// load-bearing — TopK and the predicate path both depend on it — so it is
/// preserved deliberately rather than tightened.
pub fn compare_values(a: ValueRef<'_>, b: ValueRef<'_>) -> Ordering {
    match a.rank().cmp(&b.rank()) {
        Ordering::Equal => {}
        non_equal => return non_equal,
    }
    match (a, b) {
        (ValueRef::Int(a), ValueRef::Int(b)) => a.cmp(&b),
        (ValueRef::Float(a), ValueRef::Float(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (ValueRef::Int(a), ValueRef::Float(b)) => {
            (a as f64).partial_cmp(&b).unwrap_or(Ordering::Equal)
        }
        (ValueRef::Float(a), ValueRef::Int(b)) => {
            a.partial_cmp(&(b as f64)).unwrap_or(Ordering::Equal)
        }
        (ValueRef::Str(a), ValueRef::Str(b)) => a.cmp(b),
        (ValueRef::Bool(a), ValueRef::Bool(b)) => a.cmp(&b),
        // Both Missing, both Null, or a type mismatch.
        _ => Ordering::Equal,
    }
}

/// Hash a value for use in join index lookups.
pub fn hash_value(value: ValueRef<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match value {
        // `Missing` hashes as null, matching the old behaviour where the
        // caller passed a resolved-or-null value in.
        ValueRef::Missing | ValueRef::Null => 0u8.hash(&mut hasher),
        ValueRef::Bool(b) => b.hash(&mut hasher),
        // Cast Int through f64 so Int(5) and Float(5.0) hash equally — join
        // index lookups need to match across numeric types.
        ValueRef::Int(n) => (n as f64).to_bits().hash(&mut hasher),
        ValueRef::Float(n) => n.to_bits().hash(&mut hasher),
        ValueRef::Str(s) => s.hash(&mut hasher),
        // Containers hash by kind only, as they always have: join keys are
        // scalars in practice, and hashing contents would cost a full walk on
        // the hot path for no matches gained.
        ValueRef::Arr(_) => 2u8.hash(&mut hasher),
        ValueRef::Obj(_) => 3u8.hash(&mut hasher),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::value_ref::ValueRef;
    use serde_json::json;
    use std::cmp::Ordering;

    fn v(j: serde_json::Value) -> Sp00kyValue {
        Sp00kyValue::from(j)
    }

    /// The comparison ordering is depended on by TopK's sort keys and by every
    /// predicate. Folding `Option` into `ValueRef::Missing` had to leave it
    /// exactly where it was: missing below null, null below every real value.
    #[test]
    fn missing_sorts_below_null_below_scalars() {
        let zero = v(json!(0));
        let scalar = ValueRef::from_value(&zero);
        assert_eq!(
            compare_values(ValueRef::Missing, ValueRef::Null),
            Ordering::Less
        );
        assert_eq!(
            compare_values(ValueRef::Null, ValueRef::Missing),
            Ordering::Greater
        );
        assert_eq!(compare_values(ValueRef::Null, scalar), Ordering::Less);
        assert_eq!(compare_values(scalar, ValueRef::Null), Ordering::Greater);
        assert_eq!(compare_values(ValueRef::Missing, scalar), Ordering::Less);
    }

    #[test]
    fn equal_ranks_compare_equal() {
        assert_eq!(
            compare_values(ValueRef::Missing, ValueRef::Missing),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(ValueRef::Null, ValueRef::Null),
            Ordering::Equal
        );
    }

    /// Values of different types compare Equal. This is odd, but it is the
    /// pre-existing behaviour and both TopK and the predicate path are built
    /// on it, so it is preserved deliberately rather than tightened.
    #[test]
    fn mismatched_types_compare_equal() {
        let s = v(json!("1"));
        let i = v(json!(1));
        let b = v(json!(true));
        let arr = v(json!([1]));
        let obj = v(json!({ "a": 1 }));
        for (a, b) in [(&s, &i), (&i, &b), (&s, &b), (&arr, &obj), (&i, &arr)] {
            assert_eq!(
                compare_values(ValueRef::from_value(a), ValueRef::from_value(b)),
                Ordering::Equal,
                "cross-type comparison must stay Equal"
            );
        }
    }

    #[test]
    fn numbers_compare_across_int_and_float() {
        let i = v(json!(5));
        let f = v(json!(5.0));
        let bigger = v(json!(5.5));
        assert_eq!(
            compare_values(ValueRef::from_value(&i), ValueRef::from_value(&f)),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(ValueRef::from_value(&i), ValueRef::from_value(&bigger)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(ValueRef::from_value(&bigger), ValueRef::from_value(&i)),
            Ordering::Greater
        );
    }

    /// Join index lookups rely on `Int(5)` and `Float(5.0)` hashing equally,
    /// since `compare_values` treats them as equal.
    #[test]
    fn hash_agrees_with_numeric_equality() {
        let i = v(json!(5));
        let f = v(json!(5.0));
        assert_eq!(
            hash_value(ValueRef::from_value(&i)),
            hash_value(ValueRef::from_value(&f))
        );
        // Missing and Null hash alike — the old signature could not tell them
        // apart at this call site either.
        assert_eq!(hash_value(ValueRef::Missing), hash_value(ValueRef::Null));
    }

    #[test]
    fn resolve_walks_nested_paths_and_reports_missing() {
        let row = v(json!({ "a": { "b": { "c": 7 } }, "flat": 1 }));
        let r = ValueRef::from_value(&row);
        assert_eq!(resolve_field(r, &Path::new("a.b.c")).as_i64(), Some(7));
        assert_eq!(resolve_field(r, &Path::new("flat")).as_i64(), Some(1));
        assert!(resolve_field(r, &Path::new("a.b.nope")).is_missing());
        assert!(resolve_field(r, &Path::new("nope.deeper")).is_missing());
        // Descending through a scalar is missing, not a panic.
        assert!(resolve_field(r, &Path::new("flat.deeper")).is_missing());
        assert!(resolve_field(ValueRef::Missing, &Path::new("a")).is_missing());
        // An empty path is the value itself.
        assert_eq!(resolve_field(r, &Path::new("")).get("flat").as_i64(), Some(1));
    }

    /// `<field>.id` on a record-reference string resolves to the string
    /// itself. Without it every `author.id = $auth.id` permission rule
    /// evaluates false on records the caller actually owns.
    #[test]
    fn record_reference_id_auto_dereferences() {
        let row = v(json!({ "author": "user:abc", "plain": "abc" }));
        let r = ValueRef::from_value(&row);
        assert_eq!(resolve_field(r, &Path::new("author.id")).as_str(), Some("user:abc"));
        // A string without a colon is not a record reference, so `.id` misses.
        assert!(resolve_field(r, &Path::new("plain.id")).is_missing());
        // A real nested `id` field still wins normally.
        let nested = v(json!({ "author": { "id": "inner" } }));
        assert_eq!(
            resolve_field(ValueRef::from_value(&nested), &Path::new("author.id")).as_str(),
            Some("inner")
        );
    }
}
