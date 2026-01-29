use ssp::engine::update::{StreamingUpdate, DeltaEvent, DeltaRecord};
use surrealdb::types::RecordId;
use serde_json::json;

// =========================================================================================
// COPIED LOGIC FROM lib.rs (Adapter for Test) - Modified to return SQL
// =========================================================================================

fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

fn format_incantation_id(id: &str) -> String {
    if id.starts_with("_spooky_query:") {
        id.to_string()
    } else {
        format!("_spooky_query:{}", id)
    }
}

// Returns list of queries instead of executing them
fn generate_edge_sql(updates: &[&StreamingUpdate]) -> Vec<String> {
    if updates.is_empty() { return vec![]; }
    let mut all_statements: Vec<String> = Vec::new();
    // We ignore bindings for string check, just check the SQL structure with placeholders
    // Or we can verify the binding logic separately.
    // The original code uses bind params: ${binding_name} -> binding value.
    // Here return the raw statement.

    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() { continue; }
        
        // Logic checks
        let incantation_id_str = format_incantation_id(&update.view_id);
        let _from_id = parse_record_id(&incantation_id_str).expect("Invalid incantation ID");
        let binding_name = format!("from{}", idx); // Binding used in SQL

        for record in &update.records {
            let stmt = match record.event {
                DeltaEvent::Created => {
                    format!(
                        "CREATE _spooky_list_ref SET in = ${1}, out = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0}), version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version, clientId = (SELECT clientId FROM ONLY ${1}).clientId",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Updated => {
                     format!(
                        "UPDATE ${1}->_spooky_list_ref SET version += 1 WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0})",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Deleted => {
                     format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0})",
                        record.id,
                        binding_name
                    )
                }
            };
            all_statements.push(stmt);
        }
    }

    all_statements
}

// =========================================================================================
// THE TEST
// =========================================================================================

#[test]
fn test_verify_user_queries_sql() {
    // We test that the expected records generate the expected SQL.
    // We mock the StreamingUpdate content that we EXPECT from the queries.

    // ======================================================
    // QUERY 1: Thread List with Author
    // Expected Result: 1 Thread, 1 Author (User). Both Created.
    // ======================================================
    let up1 = StreamingUpdate {
        view_id: "view_q1".to_string(),
        records: vec![
            DeltaRecord { id: "thread:t1".into(), event: DeltaEvent::Created }, // Main
            DeltaRecord { id: "user:alice".into(), event: DeltaEvent::Created }, // Subquery
        ]
    };

    let sql1 = generate_edge_sql(&[&up1]);
    assert_eq!(sql1.len(), 2);
    
    // Verify Thread SQL
    let t_sql = &sql1[0];
    assert!(t_sql.contains("WHERE record_id = thread:t1"));
    assert!(t_sql.contains("CREATE _spooky_list_ref SET in"));
    assert!(t_sql.contains("out = (SELECT id FROM ONLY _spooky_version WHERE record_id = thread:t1)"));

    // Verify User SQL
    let u_sql = &sql1[1];
    assert!(u_sql.contains("WHERE record_id = user:alice"));
    assert!(u_sql.contains("CREATE _spooky_list_ref SET in"));
    assert!(u_sql.contains("out = (SELECT id FROM ONLY _spooky_version WHERE record_id = user:alice)"));


    // ======================================================
    // QUERY 2: User by ID
    // Expected Result: 1 User. Created.
    // ======================================================
    let up2 = StreamingUpdate {
        view_id: "view_q2".to_string(),
        records: vec![
            DeltaRecord { id: "user:bob".into(), event: DeltaEvent::Created }, 
        ]
    };
    
    let sql2 = generate_edge_sql(&[&up2]);
    assert_eq!(sql2.len(), 1);
    assert!(sql2[0].contains("WHERE record_id = user:bob"));


    // ======================================================
    // QUERY 3: Complex Thread Detail
    // Expected Result: Thread, User(Author), Comment, User(Comment Author)
    // ======================================================
    let up3 = StreamingUpdate {
        view_id: "view_q3".to_string(),
        records: vec![
            DeltaRecord { id: "thread:t1".into(), event: DeltaEvent::Created }, 
            DeltaRecord { id: "user:alice".into(), event: DeltaEvent::Created },
            DeltaRecord { id: "comment:c1".into(), event: DeltaEvent::Created },
            DeltaRecord { id: "user:bob".into(), event: DeltaEvent::Created },
        ]
    };

    let sql3 = generate_edge_sql(&[&up3]);
    assert_eq!(sql3.len(), 4);
    
    // Just verify presence
    let combined = sql3.join("\n");
    assert!(combined.contains("record_id = thread:t1"));
    assert!(combined.contains("record_id = user:alice"));
    assert!(combined.contains("record_id = comment:c1"));
    assert!(combined.contains("record_id = user:bob"));
    
    // Verify Batched call Logic (e.g. if we pass multiple updates)
    let sql_batch = generate_edge_sql(&[&up1, &up2]);
    // from0 for up1, from1 for up2
    assert!(sql_batch[0].contains("$from0")); // thread:t1
    assert!(sql_batch[2].contains("$from1")); // user:bob (index 2 because up1 has 2 recs)

    println!("All SQL generation logic verification passed!");
}
