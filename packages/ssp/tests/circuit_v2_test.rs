use ssp::engine::circuit::{Circuit, Database};
use ssp::engine::operators::{Operator, Predicate, Projection};
use ssp::engine::types::{SpookyValue, Path, ZSet};
use ssp::engine::view::QueryPlan;
use ssp::engine::update::ViewUpdate;
use ssp::engine::eval::{apply_numeric_filter, NumericFilterConfig};
use serde_json::json;
use smol_str::SmolStr;

// The main integration test
#[test]
fn test_circuit_v2_ingest() {
    let mut circuit = Circuit::new();

    // 1. Register a view: SELECT name, age FROM users WHERE age > 30
    let plan = QueryPlan {
        id: "users_view".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan {
                    table: "users".into(),
                }),
                predicate: Predicate::Gt {
                    field: Path::new("age"),
                    value: json!(30),
                },
            }),
            projections: vec![
                Projection::Field {
                    name: Path::new("name"),
                },
                Projection::Field {
                    name: Path::new("age"),
                },
            ],
        },
    };

    circuit.register_view(plan, None, None);

    // 2. Ingest: "Alice", age 30. Should presumably NOT match filter.
    use ssp::engine::circuit::dto::BatchEntry;
    
    let record = BatchEntry::create(
        "users",
        "1",
        SpookyValue::from(json!({
            "name": "Alice",
            "age": 30
        }))
    );

    let updates = circuit.ingest_single(record);

    if !updates.is_empty() {
        match &updates[0] {
            ViewUpdate::Flat(update) => {
                assert!(update.result_data.is_empty(), "Expected empty result for age 30");
            }
            _ => panic!("Expected Flat view update"),
        }
    }
    
    // 4. Ingest Update: "Alice", age 31. Should match filter.
    let record_update = BatchEntry::update(
        "users",
        "1",
        SpookyValue::from(json!({
            "name": "Alice",
            "age": 31
        }))
    );
    
    let updates2 = circuit.ingest_single(record_update);
    assert_eq!(updates2.len(), 1, "Expected 1 update for age 31");
}

#[test]
fn test_numeric_filter_isolation() {
    let mut db = Database::new();
    let table_name = "users";
    let tb = db.ensure_table(table_name);
    
    let id = "1";
    let data = SpookyValue::from(json!({ "age": 31 })); // 31 > 30
    
    // Manually setup table state
    tb.rows.insert(SmolStr::new(id), data);
    
    let zset_key = SmolStr::new(format!("{}:{}", table_name, id));
    let mut input_zset = ZSet::default();
    input_zset.insert(zset_key.clone(), 1);
    
    // Setup filter config
    let path = Path::new("age");
    let predicate = Predicate::Gt { field: path.clone(), value: json!(30) };
    let config = NumericFilterConfig::from_predicate(&predicate).expect("Failed to create config");
    
    let result = apply_numeric_filter(&input_zset, &config, &db);
    
    assert_eq!(result.len(), 1, "Filter should pass");
    assert_eq!(result.get(&zset_key), Some(&1));
}
