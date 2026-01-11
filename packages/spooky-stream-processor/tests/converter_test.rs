use spooky_stream_processor::converter::convert_surql_to_dbsp;
use serde_json::json;

#[test]
fn test_simple_select_conversion() {
    let sql = "SELECT * FROM user WHERE active = true";
    let plan = convert_surql_to_dbsp(sql).expect("Failed to parse SQL");

    let expected = json!({
        "op": "filter",
        "predicate": {
            "type": "eq",
            "field": "active",
            "value": true
        },
        "input": {
            "op": "scan",
            "table": "user"
        }
    });

    assert_eq!(plan, expected);
}

#[test]
fn test_select_limit_conversion() {
    let sql = "SELECT name, age FROM person LIMIT 10";
    let plan = convert_surql_to_dbsp(sql).expect("Failed to parse SQL");

    // Expected: Limit -> Project -> Scan
    let expected = json!({
        "op": "limit",
        "limit": 10,
        "input": {
            "op": "project",
            "projections": [
                { "type": "field", "name": "name" },
                { "type": "field", "name": "age" }
            ],
            "input": {
                "op": "scan",
                "table": "person"
            }
        }
    });

    assert_eq!(plan, expected);
}
