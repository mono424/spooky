mod common;
use serde_json::Value;

#[tokio::test]
async fn test_query_vars_flow() {
    let db = common::setup_mem().await;
    db.use_db("test".to_string(), "test".to_string()).await.expect("Failed to use db");

    // 1. Test Query WITH Vars (Option<String> should be parsed)
    // ---------------------------------------------------------
    println!("Testing query WITH vars...");
    let sql_with_vars = "RETURN $val * 2;";
    let vars_json = r#"{ "val": 21 }"#;
    
    let res_with_vars = db.query(sql_with_vars.to_string(), Some(vars_json.to_string())).await.expect("Query with vars failed");
    
    // Parse result: Expect [42]
    let res_val: Vec<Value> = serde_json::from_str(&res_with_vars).expect("Failed to parse query result");
    // Parse result: Expect [{"Number": {"Int": 42}}] or similar
    let res_val: Vec<Value> = serde_json::from_str(&res_with_vars).expect("Failed to parse query result");
    assert_eq!(res_val.len(), 1);
    
    // Check deeply nested value because SurrealDB SDK returns tagged unions
    let num_val = res_val[0].get("Number").and_then(|n| n.get("Int").or(n.get("Float"))).and_then(|v| v.as_i64());
    
    if let Some(val) = num_val {
        assert_eq!(val, 42);
        println!("✅ Query WITH vars passed: {}", val);
    } else {
        // Fallback: It might be just 42 in some versions/configs, or different tag
        println!("⚠️ Unexpected format, inspecting: {:?}", res_val[0]);
        // If it was just 42, this assertion would fail above.
        // Let's assert strictly on what we saw in the failure log
        // left: Object {"Number": Object {"Int": Number(42)}}
        assert_eq!(res_val[0]["Number"]["Int"], 42);
        println!("✅ Query WITH vars passed (Verbose Format)");
    }

    // 2. Test Query WITHOUT Vars (None)
    // ---------------------------------------------------------
    println!("Testing query WITHOUT vars (None)...");
    let sql_no_vars = "RETURN 100;";
    
    let res_no_vars = db.query(sql_no_vars.to_string(), None).await.expect("Query without vars failed");
    
    // Parse result: Expect [100]
    let res_val_no: Vec<Value> = serde_json::from_str(&res_no_vars).expect("Failed to parse query result");
    assert_eq!(res_val_no.len(), 1);
    
    // Same check for 100
    if let Some(val) = res_val_no[0].get("Number").and_then(|n| n.get("Int")).and_then(|v| v.as_i64()) {
        assert_eq!(val, 100);
        println!("✅ Query WITHOUT vars passed: {}", val);
    } else {
         assert_eq!(res_val_no[0]["Number"]["Int"], 100);
         println!("✅ Query WITHOUT vars passed (Verbose Format)");
    }

    // 3. Test Transaction WITH Vars
    // ---------------------------------------------------------
    println!("Testing transaction WITH vars...");
    // Transaction statements: create a user
    let stmts = r#"["CREATE user:test SET name = $name"]"#;
    let trans_vars = r#"{ "name": "Tester" }"#;
    
    let trans_res = db.transaction(stmts.to_string(), Some(trans_vars.to_string())).await.expect("Transaction failed");
    println!("Transaction raw output: {}", trans_res);
    
    // Check if user was created
    let check_sql = "SELECT * FROM user:test";
    let check_res = db.query(check_sql.to_string(), None).await.expect("Check query failed");
    let check_val: Vec<Value> = serde_json::from_str(&check_res).expect("Failed to parse check result");
    
    // Result should look like: [{ "Object": { "id": ..., "name": { "String": "Tester" } } }]
    assert_eq!(check_val.len(), 1);
    
    // Helper to get string value from verbose JSON
    // check_val[0] is the result of the first statement, which is an Array of records
    // check_val[0]["Array"][0]["Object"]["name"]["String"]
    if let Some(name) = check_val[0].get("Array")
        .and_then(|a| a.get(0))
        .and_then(|r| r.get("Object"))
        .and_then(|o| o.get("name"))
        .and_then(|n| n.get("String"))
        .and_then(|s| s.as_str()) {
            assert_eq!(name, "Tester");
            println!("✅ Transaction passed: Created user {:?}", name);
    } else {
         // Fallback direct access
         println!("⚠️ Unexpected structure, inspecting: {:?}", check_val[0]);
         let name = &check_val[0]["Array"][0]["Object"]["name"]["String"];
         assert_eq!(name, "Tester");
         println!("✅ Transaction passed: Created user {:?}", name);
    }
}
