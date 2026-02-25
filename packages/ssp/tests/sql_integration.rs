use ssp::converter;
use ssp::operator::plan::{OperatorPlan, Projection};

#[test]
fn test_join_deserialization() {
    let sql = "SELECT * FROM comment WHERE thread.author.name = 'Admin'";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("Conversion failed");

    println!("Plan JSON: {}", serde_json::to_string_pretty(&plan_json).unwrap());

    // Attempt to deserialize into OperatorPlan
    let op: OperatorPlan = serde_json::from_value(plan_json).expect("Deserialization to OperatorPlan failed!");

    // Verify it is a Join
    if let OperatorPlan::Join { left, right, on } = op {
        println!("Successfully parsed Join!");
        println!("Left: {:?}", left);
        println!("Right: {:?}", right);
        println!("On: {:?}", on);

        // Assert keys
        assert_eq!(on.left_field.0.join("."), "thread.author.name");
        assert_eq!(on.right_field.0.join("."), "id");
    } else if let OperatorPlan::Filter { .. } = op {
        println!("Parsed as Filter (expected for literal comparison).");
    } else {
        panic!("Parsed as unexpected operator: {:?}", op);
    }
}

#[test]
fn test_explicit_join_deserialization() {
    // This should definitely trigger my __JOIN_CANDIDATE__ logic
    let sql = "SELECT * FROM comment WHERE post = post.id";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("Conversion failed");

    let op: OperatorPlan = serde_json::from_value(plan_json).expect("Deserialization to OperatorPlan failed!");

    if let OperatorPlan::Join { on, .. } = op {
        assert_eq!(on.left_field.as_str(), "post");
        assert_eq!(on.right_field.as_str(), "id");
    } else {
        panic!("Expected Join, got {:?}", op);
    }
}

#[test]
fn test_subquery_projection() {
    let sql = "SELECT id, (SELECT name FROM tags WHERE parent = id) AS tag_name FROM items";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("Conversion failed");

    let op: OperatorPlan = serde_json::from_value(plan_json).expect("Deserialization to OperatorPlan failed!");

    if let OperatorPlan::Project { projections, .. } = op {
        let has_subquery = projections.iter().any(|p| matches!(p, Projection::Subquery { .. }));
        assert!(has_subquery, "Expected Subquery projection");
    } else {
         panic!("Expected Project, got {:?}", op);
    }
}

#[test]
fn test_parse_mixed_join_and_filter_real() {
    // "thread" in comment table joins with "thread" table's "id"
    let sql = "SELECT * FROM comment WHERE thread = thread.id AND text = 'Bug'";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("Parsing failed");

    assert_eq!(plan_json["op"], "filter", "Top op is filter");
    assert_eq!(plan_json["input"]["op"], "join", "Inner op is join");

    let join_op = &plan_json["input"];
    assert_eq!(join_op["right"]["table"], "thread");
    assert_eq!(join_op["on"]["left_field"], "thread");
    assert_eq!(join_op["on"]["right_field"], "id");
}

#[test]
fn test_parse_multiple_joins() {
    // Two joins: thread = thread.id AND author = author.id
    let sql = "SELECT * FROM comment WHERE thread = thread.id AND author = author.id";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("Parsing failed");

    println!("{}", serde_json::to_string_pretty(&plan_json).unwrap());

    // Should be Join(Join(Scan))
    // The order depends on how I iterate `joins` vec.
    // But recursively one input should be another Join.

    let op1 = &plan_json;
    assert_eq!(op1["op"], "join");

    let op2 = &op1["left"];
    assert_eq!(op2["op"], "join");

    let scan = &op2["left"];
    assert_eq!(scan["op"], "scan");
}
