use dbsp_module::{converter, Operator, Predicate, Projection};
use serde_json::json;

#[test]
fn test_sql_registration() {
    let plan_sql = "SELECT * FROM test_table WHERE status = 'active'";
    let parsed = converter::convert_surql_to_dbsp(plan_sql).expect("Failed to parse SQL");
    let op: Operator = serde_json::from_value(parsed).expect("Failed to deserialize to Operator");
    
    match op {
        Operator::Filter { predicate, input } => {
            match predicate {
                Predicate::Eq { field, value } => {
                    assert_eq!(field, "status");
                    assert_eq!(value, json!("active"));
                },
                _ => panic!("Expected Eq predicate"),
            }
            match *input {
                Operator::Scan { table } => assert_eq!(table, "test_table"),
                _ => panic!("Expected Scan input"),
            }
        },
        _ => panic!("Expected Filter operator"),
    }
}

#[test]
fn test_recursive_join_registration() {
    // A -> B -> C
    // user -> post -> comment
    let plan_sql = "SELECT * FROM user, post, comment WHERE user.id = post.author AND post.id = comment.post_id AND user.status = 'active'";
    let parsed = converter::convert_surql_to_dbsp(plan_sql).expect("Failed to parse Join SQL");
    let op: Operator = serde_json::from_value(parsed).expect("Failed to deserialize");
    
    // Naive parser produces Filter(Join) because status='active' is last in WHERE.
    // We adjust expectation to handle Filter at root.
    if let Operator::Filter { input, predicate } = op {
         // Verify predicate
         if let Predicate::Eq { field, value } = predicate {
             assert_eq!(field, "status");
             assert_eq!(value, json!("active"));
         } else { panic!("Expected status=active filter at root"); }
         
         // Now check Join
         if let Operator::Join { left, right, on } = *input {
            assert_eq!(on.left_field, "id");
            assert_eq!(on.right_field, "post_id");
            
            // This join is post=comment
            // Left is Join(user, post). Right is comment.
            // Wait, standard parser builds left-deep?
            // "user, post, comment".
            // Scan(user). Join(post). Join(comment).
            // Yes.
            
            if let Operator::Join { left: l2, right: r2, on: on2 } = *left {
                 // user=post
                 assert_eq!(on2.left_field, "id");
                 assert_eq!(on2.right_field, "author");
                 if let Operator::Scan { table } = *l2 { assert_eq!(table, "user"); }
                 if let Operator::Scan { table } = *r2 { assert_eq!(table, "post"); }
            } else { panic!("Expected inner Join (user=post)"); }
            
            if let Operator::Scan { table } = *right { assert_eq!(table, "comment"); }
            
         } else { panic!("Expected Join under Filter"); }

    } else if let Operator::Join { .. } = op {
        panic!("Received Join at root, but expected Filter(Join) with naive parser");
    } else {
        panic!("Unexpected root operator: {:?}", op);
    }
}

#[test]
fn test_subquery_projection() {
    let plan_sql = "SELECT *, (SELECT * FROM comments WHERE author = $parent.id LIMIT 10) FROM users WHERE status = 'active'";
    println!("SQL: {}", plan_sql);
    let parsed = converter::convert_surql_to_dbsp(plan_sql).expect("Failed to parse Subquery SQL");
    println!("Parsed: {}", serde_json::to_string_pretty(&parsed).unwrap());

    // Expect: Project -> Filter -> Scan
    let op: Operator = serde_json::from_value(parsed).expect("Failed to deserialize");

    match op {
        Operator::Project { input, projections } => {
             // Check projections
             // 1. All
             match projections[0] {
                 Projection::All => {},
                 _ => panic!("Expected All projection first"),
             }
             // 2. Subquery
             match &projections[1] {
                 Projection::Subquery { alias, plan } => {
                     // Check alias?? Parser default uses "subquery".
                     // User didn't give ALIAS in SQL `(SELECT ...) AS my_sub`.
                     // My converter defaults to "subquery".
                     assert_eq!(alias, "subquery");
                     
                     // Check inner plan: Limit -> Filter -> Scan(comments)
                     match plan.as_ref() {
                         Operator::Limit { input: inner_input, limit } => {
                             assert_eq!(*limit, 10);
                             match inner_input.as_ref() {
                                 Operator::Filter { input: _, predicate } => {
                                     match predicate {
                                         Predicate::Eq { field, value } => {
                                             assert_eq!(field, "author");
                                             // Check Param
                                             let v = value.as_object().unwrap();
                                             // My converter normalizes to $param without decoration "parent."
                                             assert_eq!(v.get("$param").unwrap().as_str().unwrap(), "id");
                                         },
                                         _ => panic!("Expected Eq predicate in subquery"),
                                     }
                                 },
                                 _ => panic!("Expected Filter in subquery"),
                             }
                         },
                         _ => panic!("Expected Limit in subquery"),
                     }
                 },
                 _ => panic!("Expected Subquery projection"),
             }

             // Check outer input
             match *input {
                 Operator::Filter { input: outer_scan, predicate: _ } => {
                      match *outer_scan {
                          Operator::Scan { table } => assert_eq!(table, "users"),
                          _ => panic!("Expected Scan users"),
                      }
                 },
                 _ => panic!("Expected Filter users"),
             }
        },
        _ => panic!("Expected Project operator at root"),
    }
}
