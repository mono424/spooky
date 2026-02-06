use crate::engine::circuit::Database;
use crate::engine::eval::{compare_spooky_values, normalize_record_id, resolve_nested_value};
use crate::engine::types::Path;
use crate::engine::types::SpookyValue;
use crate::engine::view::View;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    Prefix { field: Path, prefix: String },
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
}

/// Resolve predicate value, handling $param references to context
fn resolve_predicate_value(value: &Value, context: Option<&SpookyValue>) -> Option<SpookyValue> {
    if let Some(obj) = value.as_object() {
        if let Some(param_path) = obj.get("$param") {
            let ctx = context?;
            let path_str = param_path.as_str().unwrap_or("");
            let effective_path = if path_str.starts_with("parent.") {
                &path_str[7..] // Strip "parent." prefix
            } else {
                path_str
            };
            let path = Path::new(effective_path);
            resolve_nested_value(Some(ctx), &path)
                .cloned()
                .map(normalize_record_id)
        } else {
            Some(SpookyValue::from(value.clone()))
        }
    } else {
        Some(SpookyValue::from(value.clone()))
    }
}

pub fn check_predicate(
    view: &View,
    pred: &Predicate,
    key: &str,
    db: &Database,
    context: Option<&SpookyValue>,
) -> bool {
    // Helper to get actual SpookyValue for comparison from the Predicate (which stores Value)

    match pred {
        Predicate::And { predicates } => predicates
            .iter()
            .all(|p| check_predicate(view, p, key, db, context)),
        Predicate::Or { predicates } => predicates
            .iter()
            .any(|p| check_predicate(view, p, key, db, context)),
        Predicate::Prefix { field, prefix } => {
            // Check if field value starts with prefix
            if field.0.len() == 1 && field.0[0] == "id" {
                return key.starts_with(prefix);
            }
            if let Some(row_val) = view.get_row_value(key, db) {
                if let Some(val) = resolve_nested_value(Some(row_val), field) {
                    if let SpookyValue::Str(s) = val {
                        return s.starts_with(prefix);
                    }
                }
            }
            false
        }
        Predicate::Eq { field, value }
        | Predicate::Neq { field, value }
        | Predicate::Gt { field, value }
        | Predicate::Gte { field, value }
        | Predicate::Lt { field, value }
        | Predicate::Lte { field, value } => {
            let target_val = resolve_predicate_value(value, context);
            if target_val.is_none() {
                return false;
            }
            let target_val = target_val.unwrap();

            // FIX: Look up actual value from row even for "id", to ensure we match
            // the canonical ID stored in the DB (which might be "table:id").
            // The previous optimization incorrectly assumed stripped ID == Row ID.
            let actual_val_opt = view
                .get_row_value(key, db)
                .and_then(|r| resolve_nested_value(Some(r), field).cloned());

            if let Some(actual_val) = actual_val_opt {
                let ord = compare_spooky_values(Some(&actual_val), Some(&target_val));
                match pred {
                    Predicate::Eq { .. } => ord == Ordering::Equal,
                    Predicate::Neq { .. } => ord != Ordering::Equal,
                    Predicate::Gt { .. } => ord == Ordering::Greater,
                    Predicate::Gte { .. } => ord == Ordering::Greater || ord == Ordering::Equal,
                    Predicate::Lt { .. } => ord == Ordering::Less,
                    Predicate::Lte { .. } => ord == Ordering::Less || ord == Ordering::Equal,
                    _ => false,
                }
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod check_predicate_tests {
    use super::*;
    use crate::engine::circuit::Database;
    use crate::engine::operators::{Operator, Predicate};
    use crate::engine::types::{FastMap, Path, SpookyValue};
    use crate::engine::view::{QueryPlan, View};
    use smol_str::SmolStr;

    // ============================================================================
    // Helper Functions
    // ============================================================================

    /// Create a simple view for predicate testing
    fn create_test_view(table: &str) -> View {
        let plan = QueryPlan {
            id: "test_view".to_string(),
            root: Operator::Scan {
                table: table.to_string(),
            },
        };
        View::new(plan, None, None)
    }

    /// Create an empty database
    fn create_empty_db() -> Database {
        Database::new()
    }

    /// Create a SpookyValue object from key-value pairs
    fn spooky_object(fields: Vec<(&str, SpookyValue)>) -> SpookyValue {
        let mut map = FastMap::default();
        for (key, value) in fields {
            map.insert(SmolStr::new(key), value);
        }
        SpookyValue::Object(map)
    }

    /// Shorthand for SpookyValue::Str
    fn spooky_str(s: &str) -> SpookyValue {
        SpookyValue::Str(SmolStr::new(s))
    }

    /// Shorthand for SpookyValue::Number
    fn spooky_num(n: f64) -> SpookyValue {
        SpookyValue::Number(n)
    }

    /// Shorthand for SpookyValue::Bool
    fn spooky_bool(b: bool) -> SpookyValue {
        SpookyValue::Bool(b)
    }

    /// Insert a record into the database and return the key
    fn insert_record(db: &mut Database, table: &str, id: &str, data: SpookyValue) -> SmolStr {
        let key = SmolStr::new(format!("{}:{}", table, id));
        let tb = db.ensure_table(table);
        tb.rows.insert(key.clone(), data);
        tb.zset.insert(key.clone(), 1);
        key
    }

    /// Create a standard test user record
    fn create_user_record(id: &str, name: &str, status: &str, age: f64) -> SpookyValue {
        spooky_object(vec![
            ("id", spooky_str(&format!("user:{}", id))),
            ("name", spooky_str(name)),
            ("status", spooky_str(status)),
            ("age", spooky_num(age)),
            ("active", spooky_bool(status == "active")),
        ])
    }

    /// Create a nested test record (for nested path testing)
    fn create_nested_record(id: &str, score: f64, level: f64) -> SpookyValue {
        spooky_object(vec![
            ("id", spooky_str(&format!("record:{}", id))),
            (
                "profile",
                spooky_object(vec![
                    (
                        "stats",
                        spooky_object(vec![
                            ("score", spooky_num(score)),
                            ("level", spooky_num(level)),
                        ]),
                    ),
                    ("verified", spooky_bool(true)),
                ]),
            ),
        ])
    }

    /// Setup a database with multiple test users
    fn setup_test_db_with_users() -> (Database, Vec<SmolStr>) {
        let mut db = create_empty_db();
        let mut keys = Vec::new();

        keys.push(insert_record(
            &mut db,
            "user",
            "1",
            create_user_record("1", "Alice", "active", 30.0),
        ));
        keys.push(insert_record(
            &mut db,
            "user",
            "2",
            create_user_record("2", "Bob", "inactive", 25.0),
        ));
        keys.push(insert_record(
            &mut db,
            "user",
            "3",
            create_user_record("3", "Charlie", "active", 35.0),
        ));
        keys.push(insert_record(
            &mut db,
            "user",
            "4",
            create_user_record("4", "Diana", "pending", 28.0),
        ));

        (db, keys)
    }

    /// Setup a database with nested records
    fn setup_test_db_with_nested() -> (Database, Vec<SmolStr>) {
        let mut db = create_empty_db();
        let mut keys = Vec::new();

        keys.push(insert_record(
            &mut db,
            "record",
            "1",
            create_nested_record("1", 100.0, 5.0),
        ));
        keys.push(insert_record(
            &mut db,
            "record",
            "2",
            create_nested_record("2", 50.0, 3.0),
        ));
        keys.push(insert_record(
            &mut db,
            "record",
            "3",
            create_nested_record("3", 200.0, 10.0),
        ));

        (db, keys)
    }

    /// Helper to check predicate and return bool
    fn check(view: &View, pred: &Predicate, key: &SmolStr, db: &Database) -> bool {
        check_predicate(view, pred, key, db, None)
    }

    /// Helper to check predicate with context
    fn check_with_ctx(
        view: &View,
        pred: &Predicate,
        key: &SmolStr,
        db: &Database,
        ctx: &SpookyValue,
    ) -> bool {
        check_predicate(view, pred, key, db, Some(ctx))
    }

    // ============================================================================
    // Predicate Builders (for cleaner test code)
    // ============================================================================

    fn pred_eq(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Eq {
            field: Path::new(field),
            value,
        }
    }

    fn pred_neq(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Neq {
            field: Path::new(field),
            value,
        }
    }

    fn pred_gt(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Gt {
            field: Path::new(field),
            value,
        }
    }

    fn pred_gte(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Gte {
            field: Path::new(field),
            value,
        }
    }

    fn pred_lt(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Lt {
            field: Path::new(field),
            value,
        }
    }

    fn pred_lte(field: &str, value: serde_json::Value) -> Predicate {
        Predicate::Lte {
            field: Path::new(field),
            value,
        }
    }

    fn pred_prefix(field: &str, prefix: &str) -> Predicate {
        Predicate::Prefix {
            field: Path::new(field),
            prefix: prefix.to_string(),
        }
    }

    fn pred_and(predicates: Vec<Predicate>) -> Predicate {
        Predicate::And { predicates }
    }

    fn pred_or(predicates: Vec<Predicate>) -> Predicate {
        Predicate::Or { predicates }
    }

    // ============================================================================
    // Tests
    // ============================================================================

    #[test]
    fn test_predicate_eq_string_match() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Alice has status = "active"
        let pred = pred_eq("status", serde_json::json!("active"));
        assert!(check(&view, &pred, &keys[0], &db)); // Alice: active ✓

        // Bob has status = "inactive"
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: inactive ✗

        // Case sensitive
        let pred_case = pred_eq("status", serde_json::json!("ACTIVE"));
        assert!(!check(&view, &pred_case, &keys[0], &db));
    }

    #[test]
    fn test_predicate_eq_number() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_eq("age", serde_json::json!(30.0));
        assert!(check(&view, &pred, &keys[0], &db)); // Alice: 30 ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: 25 ✗
    }

    #[test]
    fn test_predicate_neq() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_neq("status", serde_json::json!("active"));
        assert!(!check(&view, &pred, &keys[0], &db)); // Alice: active = active ✗
        assert!(check(&view, &pred, &keys[1], &db)); // Bob: inactive != active ✓
    }

    #[test]
    fn test_predicate_gt_number() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_gt("age", serde_json::json!(28.0));
        assert!(check(&view, &pred, &keys[0], &db)); // Alice: 30 > 28 ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: 25 > 28 ✗
        assert!(check(&view, &pred, &keys[2], &db)); // Charlie: 35 > 28 ✓
        assert!(!check(&view, &pred, &keys[3], &db)); // Diana: 28 > 28 ✗
    }

    #[test]
    fn test_predicate_gte_number() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_gte("age", serde_json::json!(28.0));
        assert!(check(&view, &pred, &keys[0], &db)); // Alice: 30 >= 28 ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: 25 >= 28 ✗
        assert!(check(&view, &pred, &keys[2], &db)); // Charlie: 35 >= 28 ✓
        assert!(check(&view, &pred, &keys[3], &db)); // Diana: 28 >= 28 ✓
    }

    #[test]
    fn test_predicate_lt_number() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_lt("age", serde_json::json!(30.0));
        assert!(!check(&view, &pred, &keys[0], &db)); // Alice: 30 < 30 ✗
        assert!(check(&view, &pred, &keys[1], &db)); // Bob: 25 < 30 ✓
        assert!(!check(&view, &pred, &keys[2], &db)); // Charlie: 35 < 30 ✗
        assert!(check(&view, &pred, &keys[3], &db)); // Diana: 28 < 30 ✓
    }

    #[test]
    fn test_predicate_lte_number() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        let pred = pred_lte("age", serde_json::json!(30.0));
        assert!(check(&view, &pred, &keys[0], &db)); // Alice: 30 <= 30 ✓
        assert!(check(&view, &pred, &keys[1], &db)); // Bob: 25 <= 30 ✓
        assert!(!check(&view, &pred, &keys[2], &db)); // Charlie: 35 <= 30 ✗
        assert!(check(&view, &pred, &keys[3], &db)); // Diana: 28 <= 30 ✓
    }

    #[test]
    fn test_predicate_prefix_match() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Names starting with "A"
        let pred = pred_prefix("name", "A");
        assert!(check(&view, &pred, &keys[0], &db)); // Alice ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob ✗
        assert!(!check(&view, &pred, &keys[2], &db)); // Charlie ✗
        assert!(!check(&view, &pred, &keys[3], &db)); // Diana ✗

        // Names starting with "Ch"
        let pred_ch = pred_prefix("name", "Ch");
        assert!(check(&view, &pred_ch, &keys[2], &db)); // Charlie ✓
    }

    #[test]
    fn test_predicate_prefix_on_id_field() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Prefix on "id" field uses key directly
        let pred = pred_prefix("id", "user:");
        assert!(check(&view, &pred, &keys[0], &db));
        assert!(check(&view, &pred, &keys[1], &db));
    }

    #[test]
    fn test_predicate_and_all_true() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Alice: status=active AND age=30
        let pred = pred_and(vec![
            pred_eq("status", serde_json::json!("active")),
            pred_eq("age", serde_json::json!(30.0)),
        ]);
        assert!(check(&view, &pred, &keys[0], &db)); // Alice ✓
    }

    #[test]
    fn test_predicate_and_one_false() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Bob: status=active (false) AND age=25 (true)
        let pred = pred_and(vec![
            pred_eq("status", serde_json::json!("active")),
            pred_eq("age", serde_json::json!(25.0)),
        ]);
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: inactive ✗
    }

    #[test]
    fn test_predicate_or_one_true() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Bob: status=active (false) OR age=25 (true)
        let pred = pred_or(vec![
            pred_eq("status", serde_json::json!("active")),
            pred_eq("age", serde_json::json!(25.0)),
        ]);
        assert!(check(&view, &pred, &keys[1], &db)); // One is true ✓
    }

    #[test]
    fn test_predicate_or_all_false() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Bob: status=active (false) OR age=99 (false)
        let pred = pred_or(vec![
            pred_eq("status", serde_json::json!("active")),
            pred_eq("age", serde_json::json!(99.0)),
        ]);
        assert!(!check(&view, &pred, &keys[1], &db)); // All false ✗
    }

    #[test]
    fn test_predicate_nested_and_or() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // (status=active AND age>25) OR (status=pending)
        let pred = pred_or(vec![
            pred_and(vec![
                pred_eq("status", serde_json::json!("active")),
                pred_gt("age", serde_json::json!(25.0)),
            ]),
            pred_eq("status", serde_json::json!("pending")),
        ]);

        assert!(check(&view, &pred, &keys[0], &db)); // Alice: active AND 30>25 ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // Bob: inactive AND 25>25 ✗, not pending ✗
        assert!(check(&view, &pred, &keys[2], &db)); // Charlie: active AND 35>25 ✓
        assert!(check(&view, &pred, &keys[3], &db)); // Diana: pending ✓
    }

    #[test]
    fn test_predicate_nested_path() {
        let (db, keys) = setup_test_db_with_nested();
        let view = create_test_view("record");

        // Nested path: profile.stats.score > 75
        let pred = pred_gt("profile.stats.score", serde_json::json!(75.0));
        assert!(check(&view, &pred, &keys[0], &db)); // score: 100 ✓
        assert!(!check(&view, &pred, &keys[1], &db)); // score: 50 ✗
        assert!(check(&view, &pred, &keys[2], &db)); // score: 200 ✓
    }

    #[test]
    fn test_predicate_with_param_context() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Create context with parent.target_age = 30
        let ctx = spooky_object(vec![("target_age", spooky_num(30.0))]);

        // age = $parent.target_age
        let pred = Predicate::Eq {
            field: Path::new("age"),
            value: serde_json::json!({"$param": "parent.target_age"}),
        };

        assert!(check_with_ctx(&view, &pred, &keys[0], &db, &ctx)); // Alice: 30 == 30 ✓
        assert!(!check_with_ctx(&view, &pred, &keys[1], &db, &ctx)); // Bob: 25 == 30 ✗
    }

    #[test]
    fn test_predicate_missing_field() {
        let (db, keys) = setup_test_db_with_users();
        let view = create_test_view("user");

        // Field that doesn't exist
        let pred = pred_eq("nonexistent", serde_json::json!("value"));
        assert!(!check(&view, &pred, &keys[0], &db));

        // Nested path that doesn't exist
        let pred_nested = pred_eq("a.b.c.d", serde_json::json!("value"));
        assert!(!check(&view, &pred_nested, &keys[0], &db));
    }

    #[test]
    fn test_predicate_missing_record() {
        let db = create_empty_db();
        let view = create_test_view("user");
        let fake_key = SmolStr::new("user:nonexistent");

        let pred = pred_eq("status", serde_json::json!("active"));
        assert!(!check(&view, &pred, &fake_key, &db));
    }
}
