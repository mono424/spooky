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

// End-to-end regression for the production "comments vanish" bug, using the
// REAL thread-detail query shape: a WINDOWED reverse one-to-many `comments`
// subquery (ORDER BY + LIMIT) that ALSO carries a NESTED `author` subquery,
// alongside a plain forward `author` subquery. The comment must be emitted in
// the initial snapshot's `subquery_items` (→ a `_00_list_ref` edge), or clients
// never receive it. Prior unit tests only covered a *simple* reverse subquery
// and passed while production still dropped comments.
#[test]
fn windowed_nested_comments_subquery_emits_child_edge_end_to_end() {
    use ssp::circuit::{Circuit, Record};
    use ssp::operator::plan::QueryPlan;
    use serde_json::json;

    let sql = "SELECT *, \
        (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, \
        (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author \
         FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments \
        FROM thread";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("conversion failed");
    let root: OperatorPlan =
        serde_json::from_value(plan_json).expect("deserialize to OperatorPlan failed");
    let plan = QueryPlan { id: "detail".to_string(), root };

    let mut circuit = Circuit::new();
    circuit.load(vec![
        Record::new("user", "user:u", json!({ "username": "alice" })),
        Record::new("thread", "thread:t", json!({ "title": "Hello", "author": "user:u" })),
        Record::new(
            "comment",
            "comment:c",
            json!({ "text": "hi", "thread": "thread:t", "author": "user:u", "created_at": "2026-01-01T00:00:00Z" }),
        ),
    ]);

    let delta = circuit
        .add_query(plan, None, None)
        .expect("registration must yield an initial delta");

    let has_comment = delta.subquery_items.iter().any(|it| it.id == "comment:c");
    let has_author = delta.subquery_items.iter().any(|it| it.id == "user:u");
    assert!(
        has_comment,
        "the windowed+nested `comments` subquery must emit comment:c as a subquery_item \
         (author present={has_author}) — else the SSP writes no _00_list_ref edge and \
         comments vanish on the client. subquery_items: {:?}",
        delta.subquery_items
    );
}

// The EXACT production thread-detail query: `WHERE id = $id LIMIT 1` parent
// filter, a forward `author`, a windowed reverse `comments` with a nested
// `author`, and a `jobs` subquery. Reproduces the live case where authors
// emit but comments don't. The `comments` child must appear in subquery_items.
#[test]
fn real_thread_detail_query_emits_comment_edge() {
    use ssp::circuit::{Circuit, Record};
    use ssp::operator::plan::QueryPlan;
    use serde_json::json;

    let sql = "SELECT *, \
        (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, \
        (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author \
         FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments, \
        (SELECT * FROM job WHERE assigned_to=$parent.id AND path = '/spookify' \
         ORDER BY created_at desc LIMIT 1) AS jobs \
        FROM thread";
    let plan_json = converter::convert_surql_to_dbsp(sql).expect("conversion failed");
    let root: OperatorPlan = serde_json::from_value(plan_json).expect("deserialize failed");
    let plan = QueryPlan { id: "detail".to_string(), root };

    let mut c = Circuit::new();
    c.load(vec![
        Record::new("user", "user:u", json!({ "username": "alice" })),
        Record::new("thread", "thread:t", json!({ "title": "Hello", "author": "user:u" })),
        Record::new(
            "comment",
            "comment:c",
            json!({ "text": "hi", "thread": "thread:t", "author": "user:u", "created_at": "2026-01-01T00:00:00Z" }),
        ),
    ]);

    let delta = c
        .add_query(plan, None, None)
        .expect("registration must yield an initial delta");

    let items: Vec<_> = delta.subquery_items.iter().map(|it| (it.id.clone(), it.alias.clone())).collect();
    assert!(
        delta.subquery_items.iter().any(|it| it.id == "comment:c"),
        "the real detail query must emit comment:c in subquery_items — items: {:?}",
        items
    );
}
