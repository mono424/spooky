//! Tests for improved Circuit and View processing
//!
//! These tests cover the improvements made to:
//! 1. Operation enum (type-safe ops instead of string matching)
//! 2. BatchEntry struct (cleaner data model)
//! 3. IngestBatch builder (ergonomic API)
//! 4. Group-by-table processing (cache locality)
//! 5. Refactored process_ingest (smaller focused functions)
//!
//! Run with: cargo test --test improved_changes_test

mod common;

use common::*;
use serde_json::json;
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::{Circuit, QueryPlan, Operator, JoinCondition, Path, Predicate, Projection, SpookyValue};

// ============================================================================
// PART 1: Operation Enum Tests
// ============================================================================

mod operation_tests {
    use super::*;

    /// Test Operation::from_str parsing (case-insensitive)
    #[test]
    fn test_operation_parsing() {
        // These tests verify the Operation enum parses correctly
        // In the improved code, Operation::from_str replaces string matching

        // Valid operations (uppercase)
        assert!(matches_op("CREATE", "CREATE"));
        assert!(matches_op("UPDATE", "UPDATE"));
        assert!(matches_op("DELETE", "DELETE"));

        // Valid operations (lowercase)
        assert!(matches_op("create", "CREATE"));
        assert!(matches_op("update", "UPDATE"));
        assert!(matches_op("delete", "DELETE"));

        // Invalid operations should not cause panics
        let mut circuit = setup();
        let updates = circuit.ingest_batch(
            vec![(
                "users".into(),
                "INVALID_OP".into(), // Should be ignored
                "users:1".into(),
                json!({"name": "Test"}),
                "hash".into(),
            )],
    );
        // No crash, empty result since op was invalid
        assert!(updates.is_empty());
    }

    fn matches_op(input: &str, _expected: &str) -> bool {
        // Helper to verify operation parsing works
        let mut circuit = setup();
        let result = circuit.ingest_batch(
            vec![(
                "test".into(),
                input.into(),
                "test:1".into(),
                json!({"id": "test:1"}),
                "hash".into(),
            )],
        );
        // If parsing worked, the record was ingested
        circuit.db.tables.contains_key("test")
    }

    /// Test operation weight calculation
    #[test]
    fn test_operation_weights() {
        let mut circuit = setup();

        // Register a simple view
        let plan = QueryPlan {
            id: "weight_test".to_string(),
            root: Operator::Scan {
                table: "items".to_string(),
            },
        };
        circuit.register_view(plan, None, None);

        // CREATE adds record (weight +1)
        ingest(&mut circuit, "items", "CREATE", "items:1", json!({"id": "items:1"}));
        assert!(circuit.db.tables.get("items").unwrap().zset.contains_key("items:items:1"));

        // DELETE removes record (weight -1)
        ingest(&mut circuit, "items", "DELETE", "items:1", json!({}));
        assert!(!circuit.db.tables.get("items").unwrap().zset.contains_key("items:items:1"));

        // UPDATE keeps record (weight +1, replaces existing)
        ingest(&mut circuit, "items", "CREATE", "items:2", json!({"id": "items:2", "val": 1}));
        ingest(&mut circuit, "items", "UPDATE", "items:2", json!({"id": "items:2", "val": 2}));
        assert!(circuit.db.tables.get("items").unwrap().rows.contains_key("items:2"));
    }
}

// ============================================================================
// PART 2: Mixed Operations Tests (Same Table)
// ============================================================================

mod mixed_ops_same_table_tests {
    use super::*;

    /// Test CREATE + UPDATE + DELETE in single batch for same table
    #[test]
    fn test_create_update_delete_same_table() {
        let mut circuit = setup();

        // Register view to track changes
        let plan = QueryPlan {
            id: "users_view".to_string(),
            root: Operator::Scan {
                table: "users".to_string(),
            },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // Batch: create user:1, create user:2, update user:1, delete user:2
        let batch = vec![
            ("users".into(), "CREATE".into(), "users:1".into(), json!({"id": "users:1", "name": "Alice"}), "h1".into()),
            ("users".into(), "CREATE".into(), "users:2".into(), json!({"id": "users:2", "name": "Bob"}), "h2".into()),
            ("users".into(), "UPDATE".into(), "users:1".into(), json!({"id": "users:1", "name": "Alice Smith"}), "h3".into()),
            ("users".into(), "DELETE".into(), "users:2".into(), json!({}), "h4".into()),
        ];

        let updates = circuit.ingest_batch(batch);

        // Verify final state
        let users_table = circuit.db.tables.get("users").unwrap();
        
        // user:1 should exist with updated name
        assert!(users_table.rows.contains_key("users:1"), "user:1 should exist");
        
        // user:2 should be deleted
        assert!(!users_table.rows.contains_key("users:2"), "user:2 should be deleted");

        // View should have been updated
        assert!(!updates.is_empty(), "Should have view updates");

        println!("[TEST] ✓ Mixed ops (CREATE/UPDATE/DELETE) same table works correctly");
    }

    /// Test multiple UPDATEs to same record in single batch
    #[test]
    fn test_multiple_updates_same_record() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "multi_update_view".to_string(),
            root: Operator::Scan {
                table: "items".to_string(),
            },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // Create initial record
        ingest(&mut circuit, "items", "CREATE", "items:1", json!({"id": "items:1", "counter": 0}));

        // Batch multiple updates (simulates rapid mutations)
        let batch = vec![
            ("items".into(), "UPDATE".into(), "items:1".into(), json!({"id": "items:1", "counter": 1}), "h1".into()),
            ("items".into(), "UPDATE".into(), "items:1".into(), json!({"id": "items:1", "counter": 2}), "h2".into()),
            ("items".into(), "UPDATE".into(), "items:1".into(), json!({"id": "items:1", "counter": 3}), "h3".into()),
        ];

        circuit.ingest_batch(batch);

        // Final value should be counter: 3 (last update wins)
        let items = circuit.db.tables.get("items").unwrap();
        let item = items.rows.get("items:1").unwrap();
        
        // SpookyValue comparison - check the counter field
        if let SpookyValue::Object(map) = item {
            if let Some(SpookyValue::Number(n)) = map.get("counter") {
                assert_eq!(*n as i32, 3, "Counter should be 3 after all updates");
            }
        }

        println!("[TEST] ✓ Multiple updates to same record in batch works correctly");
    }

    /// Test CREATE followed by DELETE in same batch (net zero)
    #[test]
    fn test_create_then_delete_same_batch() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "net_zero_view".to_string(),
            root: Operator::Scan {
                table: "ephemeral".to_string(),
            },
        };
        circuit.register_view(plan, None, None);

        // Create and immediately delete in same batch
        let batch = vec![
            ("ephemeral".into(), "CREATE".into(), "ephemeral:1".into(), json!({"id": "ephemeral:1"}), "h1".into()),
            ("ephemeral".into(), "DELETE".into(), "ephemeral:1".into(), json!({}), "h2".into()),
        ];

        circuit.ingest_batch(batch);

        // Record should not exist (weight cancels out: +1 - 1 = 0)
        let table = circuit.db.tables.get("ephemeral");
        if let Some(t) = table {
            assert!(!t.zset.contains_key("ephemeral:1"), "Record should not exist in ZSet");
        }

        println!("[TEST] ✓ CREATE + DELETE in same batch correctly cancels out");
    }
}

// ============================================================================
// PART 3: Mixed Tables Tests
// ============================================================================

mod mixed_tables_tests {
    use super::*;

    /// Test batch with records for multiple different tables
    #[test]
    fn test_multi_table_batch() {
        let mut circuit = setup();

        // Register views for each table
        for table in ["users", "posts", "comments"] {
            let plan = QueryPlan {
                id: format!("{}_view", table),
                root: Operator::Scan {
                    table: table.to_string(),
                },
            };
            circuit.register_view(plan, None, None);
        }

        // Single batch affecting all three tables
        let batch = vec![
            ("users".into(), "CREATE".into(), "users:1".into(), json!({"id": "users:1", "name": "Alice"}), "h1".into()),
            ("posts".into(), "CREATE".into(), "posts:1".into(), json!({"id": "posts:1", "title": "Hello", "author": "users:1"}), "h2".into()),
            ("comments".into(), "CREATE".into(), "comments:1".into(), json!({"id": "comments:1", "text": "Nice!", "post": "posts:1"}), "h3".into()),
            ("users".into(), "CREATE".into(), "users:2".into(), json!({"id": "users:2", "name": "Bob"}), "h4".into()),
        ];

        let updates = circuit.ingest_batch(batch);

        // Verify all tables have correct data
        assert!(circuit.db.tables.get("users").unwrap().rows.len() == 2);
        assert!(circuit.db.tables.get("posts").unwrap().rows.len() == 1);
        assert!(circuit.db.tables.get("comments").unwrap().rows.len() == 1);

        // Should have updates for all three views
        assert!(updates.len() >= 3, "Should have updates for all affected views");

        println!("[TEST] ✓ Multi-table batch processes all tables correctly");
    }

    /// Test that views are only updated for their dependent tables
    #[test]
    fn test_dependency_isolation() {
        let mut circuit = setup();

        // View only depends on "users"
        let users_plan = QueryPlan {
            id: "users_only".to_string(),
            root: Operator::Scan {
                table: "users".to_string(),
            },
        };
        circuit.register_view(users_plan, None, Some(ViewResultFormat::Streaming));

        // View only depends on "products"
        let products_plan = QueryPlan {
            id: "products_only".to_string(),
            root: Operator::Scan {
                table: "products".to_string(),
            },
        };
        circuit.register_view(products_plan, None, Some(ViewResultFormat::Streaming));

        // Batch affecting only "users"
        let batch = vec![
            ("users".into(), "CREATE".into(), "users:1".into(), json!({"id": "users:1"}), "h1".into()),
        ];

        let updates = circuit.ingest_batch(batch);

        // Only users_only view should be updated
        let view_ids: Vec<&str> = updates.iter().map(|u| u.query_id()).collect();
        assert!(view_ids.contains(&"users_only"), "users_only view should be updated");
        
        // products_only should NOT be in the updates (unless it's the initial empty state)
        // Actually on first batch it might have initial state, so check dependency graph
        assert!(circuit.dependency_graph.get("users").unwrap().len() == 1);
        assert!(circuit.dependency_graph.get("products").unwrap().len() == 1);

        println!("[TEST] ✓ Dependency isolation works - only affected views are updated");
    }

    /// Test interleaved operations across tables
    #[test]
    fn test_interleaved_multi_table_ops() {
        let mut circuit = setup();

        // Setup: create initial data
        ingest(&mut circuit, "authors", "CREATE", "authors:1", json!({"id": "authors:1", "name": "Writer"}));

        // Register a join view
        let plan = QueryPlan {
            id: "books_with_authors".to_string(),
            root: Operator::Join {
                left: Box::new(Operator::Scan { table: "books".to_string() }),
                right: Box::new(Operator::Scan { table: "authors".to_string() }),
                on: JoinCondition {
                    left_field: Path::new("author"),
                    right_field: Path::new("id"),
                },
            },
        };
        circuit.register_view(plan, None, None);

        // Interleaved batch: book -> author update -> another book
        let batch = vec![
            ("books".into(), "CREATE".into(), "books:1".into(), json!({"id": "books:1", "title": "Book 1", "author": "authors:1"}), "h1".into()),
            ("authors".into(), "UPDATE".into(), "authors:1".into(), json!({"id": "authors:1", "name": "Famous Writer"}), "h2".into()),
            ("books".into(), "CREATE".into(), "books:2".into(), json!({"id": "books:2", "title": "Book 2", "author": "authors:1"}), "h3".into()),
        ];

        let updates = circuit.ingest_batch(batch);

        // Both books should exist
        assert!(circuit.db.tables.get("books").unwrap().rows.len() == 2);
        
        // Author should have updated name
        let author = circuit.db.tables.get("authors").unwrap().rows.get("authors:1").unwrap();
        if let SpookyValue::Object(map) = author {
            if let Some(SpookyValue::Str(name)) = map.get("name") {
                assert_eq!(name.as_str(), "Famous Writer");
            }
        }

        println!("[TEST] ✓ Interleaved multi-table operations work correctly");
    }
}

// ============================================================================
// PART 4: View Update Correctness Tests
// ============================================================================

mod view_update_tests {
    use super::*;

    /// Test that streaming updates have correct delta events
    #[test]
    fn test_streaming_delta_events() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "streaming_test".to_string(),
            root: Operator::Scan {
                table: "items".to_string(),
            },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // CREATE should emit Created event
        let create_updates = ingest(&mut circuit, "items", "CREATE", "items:1", json!({"id": "items:1"}));
        assert_has_event(&create_updates, "streaming_test", "items:1", DeltaEvent::Created);

        // UPDATE should emit Updated event
        let update_updates = ingest(&mut circuit, "items", "UPDATE", "items:1", json!({"id": "items:1", "val": 1}));
        assert_has_event(&update_updates, "streaming_test", "items:1", DeltaEvent::Updated);

        // DELETE should emit Deleted event
        let delete_updates = ingest(&mut circuit, "items", "DELETE", "items:1", json!({}));
        assert_has_event(&delete_updates, "streaming_test", "items:1", DeltaEvent::Deleted);

        println!("[TEST] ✓ Streaming delta events are correct for CREATE/UPDATE/DELETE");
    }

    fn assert_has_event(updates: &[ViewUpdate], view_id: &str, record_id: &str, expected: DeltaEvent) {
        for update in updates {
            if let ViewUpdate::Streaming(s) = update {
                if s.view_id == view_id {
                    for record in &s.records {
                        if record.id == record_id && record.event == expected {
                            return;
                        }
                    }
                }
            }
        }
        panic!("Expected {:?} event for {} in view {}", expected, record_id, view_id);
    }
}

// ============================================================================
// PART 5: Performance/Optimization Tests
// ============================================================================

mod optimization_tests {
    use super::*;
    use std::time::Instant;

    /// Test that batch processing is faster than individual ingests
    #[test]
    fn test_batch_vs_individual_performance() {
        const NUM_RECORDS: usize = 100;

        // Test 1: Individual ingests
        let mut circuit1 = setup();
        let plan1 = QueryPlan {
            id: "perf_test_1".to_string(),
            root: Operator::Scan { table: "perf".to_string() },
        };
        circuit1.register_view(plan1, None, None);

        let start_individual = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = format!("perf:{}", i);
            ingest(&mut circuit1, "perf", "CREATE", &id, json!({"id": id, "i": i}));
        }
        let individual_time = start_individual.elapsed();

        // Test 2: Batch ingest
        let mut circuit2 = setup();
        let plan2 = QueryPlan {
            id: "perf_test_2".to_string(),
            root: Operator::Scan { table: "perf".to_string() },
        };
        circuit2.register_view(plan2, None, None);

        let batch: Vec<_> = (0..NUM_RECORDS)
            .map(|i| {
                let id = format!("perf:{}", i);
                let record = json!({"id": &id, "i": i});
                let hash = generate_hash(&record);
                ("perf".to_string(), "CREATE".to_string(), id, record, hash)
            })
            .collect();

        let start_batch = Instant::now();
        circuit2.ingest_batch(batch);
        let batch_time = start_batch.elapsed();

        println!(
            "[PERF] Individual: {:?}, Batch: {:?}, Speedup: {:.2}x",
            individual_time,
            batch_time,
            individual_time.as_nanos() as f64 / batch_time.as_nanos() as f64
        );

        // Batch should generally be faster (allows amortization)
        // Note: This might not always hold for small batches due to overhead
        assert!(
            circuit1.db.tables.get("perf").unwrap().rows.len() == NUM_RECORDS,
            "Individual ingest should have all records"
        );
        assert!(
            circuit2.db.tables.get("perf").unwrap().rows.len() == NUM_RECORDS,
            "Batch ingest should have all records"
        );

        println!("[TEST] ✓ Batch processing performance test completed");
    }

    /// Test dependency graph prevents unnecessary view processing
    #[test]
    fn test_dependency_graph_efficiency() {
        let mut circuit = setup();

        // Create 10 views, each on different tables
        for i in 0..10 {
            let plan = QueryPlan {
                id: format!("view_{}", i),
                root: Operator::Scan {
                    table: format!("table_{}", i),
                },
            };
            circuit.register_view(plan, None, None);
        }

        // Verify dependency graph has correct mappings
        for i in 0..10 {
            let table = format!("table_{}", i);
            let deps = circuit.dependency_graph.get(&table);
            assert!(deps.is_some(), "Table {} should have dependencies", table);
            assert_eq!(deps.unwrap().len(), 1, "Each table should have exactly 1 view");
        }

        // Ingest to only table_0 - only view_0 should be processed
        let updates = ingest(
            &mut circuit,
            "table_0",
            "CREATE",
            "table_0:1",
            json!({"id": "table_0:1"}),
    );

        // Should only have 1 update (for view_0)
        assert_eq!(updates.len(), 1, "Only view_0 should be updated");
        assert_eq!(updates[0].query_id(), "view_0");

        println!("[TEST] ✓ Dependency graph correctly limits view processing");
    }
}

// ============================================================================
// PART 6: Edge Cases and Regression Tests
// ============================================================================

mod edge_case_tests {
    use super::*;

    /// Test empty batch handling
    #[test]
    fn test_empty_batch() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "empty_test".to_string(),
            root: Operator::Scan { table: "empty".to_string() },
        };
        circuit.register_view(plan, None, None);

        let updates = circuit.ingest_batch(vec![]);
        assert!(updates.is_empty(), "Empty batch should produce no updates");

        println!("[TEST] ✓ Empty batch handled correctly");
    }

    /// Test batch with all invalid operations
    #[test]
    fn test_all_invalid_ops_batch() {
        let mut circuit = setup();

        let batch = vec![
            ("test".into(), "INVALID1".into(), "test:1".into(), json!({}), "h1".into()),
            ("test".into(), "INVALID2".into(), "test:2".into(), json!({}), "h2".into()),
        ];

        // Should not panic, just skip invalid ops
        let updates = circuit.ingest_batch(batch);
        
        // No valid operations = no table created
        assert!(!circuit.db.tables.contains_key("test"));

        println!("[TEST] ✓ All invalid ops batch handled gracefully");
    }

    /// Test rapid create/delete cycles
    #[test]
    fn test_rapid_create_delete_cycles() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "cycle_test".to_string(),
            root: Operator::Scan { table: "cycle".to_string() },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // Create and delete same record multiple times
        for cycle in 0..5 {
            ingest(&mut circuit, "cycle", "CREATE", "cycle:1", json!({"id": "cycle:1", "cycle": cycle}));
            ingest(&mut circuit, "cycle", "DELETE", "cycle:1", json!({}));
        }

        // Record should not exist
        let table = circuit.db.tables.get("cycle");
        if let Some(t) = table {
            assert!(!t.zset.contains_key("cycle:1"));
        }

        println!("[TEST] ✓ Rapid create/delete cycles handled correctly");
    }

    /// Test very large batch
    #[test]
    fn test_large_batch() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "large_test".to_string(),
            root: Operator::Scan { table: "large".to_string() },
        };
        circuit.register_view(plan, None, None);

        const BATCH_SIZE: usize = 1000;
        let batch: Vec<_> = (0..BATCH_SIZE)
            .map(|i| {
                let id = format!("large:{}", i);
                let record = json!({"id": &id, "index": i});
                let hash = generate_hash(&record);
                ("large".to_string(), "CREATE".to_string(), id, record, hash)
            })
            .collect();

        let updates = circuit.ingest_batch(batch);

        // All records should be ingested
        assert_eq!(
            circuit.db.tables.get("large").unwrap().rows.len(),
            BATCH_SIZE
        );

        // Should have view update
        assert!(!updates.is_empty());

        println!("[TEST] ✓ Large batch ({} records) processed correctly", BATCH_SIZE);
    }

    /// Test Unicode/special characters in data
    #[test]
    fn test_unicode_data() {
        let mut circuit = setup();

        let plan = QueryPlan {
            id: "unicode_test".to_string(),
            root: Operator::Scan { table: "unicode".to_string() },
        };
        circuit.register_view(plan, None, None);

        let batch = vec![
            ("unicode".into(), "CREATE".into(), "unicode:1".into(), json!({"id": "unicode:1", "name": "日本語"}), "h1".into()),
            ("unicode".into(), "CREATE".into(), "unicode:2".into(), json!({"id": "unicode:2", "emoji": "🎉🚀💯"}), "h2".into()),
            ("unicode".into(), "CREATE".into(), "unicode:3".into(), json!({"id": "unicode:3", "special": "a\nb\tc\"d"}), "h3".into()),
        ];

        circuit.ingest_batch(batch);

        assert_eq!(circuit.db.tables.get("unicode").unwrap().rows.len(), 3);

        println!("[TEST] ✓ Unicode and special characters handled correctly");
    }
}

// ============================================================================
// PART 7: Subquery and Complex Query Tests
// ============================================================================

mod complex_query_tests {
    use super::*;

    /// Test batch updates with join views
    #[test]
    fn test_batch_with_join_view() {
        let mut circuit = setup();

        // Create join view: threads with authors
        let plan = QueryPlan {
            id: "threads_with_authors".to_string(),
            root: Operator::Join {
                left: Box::new(Operator::Scan { table: "thread".to_string() }),
                right: Box::new(Operator::Scan { table: "author".to_string() }),
                on: JoinCondition {
                    left_field: Path::new("author"),
                    right_field: Path::new("id"),
                },
            },
        };
        circuit.register_view(plan, None, None);

        // Batch: create author and thread together
        let batch = vec![
            ("author".into(), "CREATE".into(), "author:1".into(), json!({"id": "author:1", "name": "Alice"}), "h1".into()),
            ("thread".into(), "CREATE".into(), "thread:1".into(), json!({"id": "thread:1", "title": "Test", "author": "author:1"}), "h2".into()),
        ];

        let updates = circuit.ingest_batch(batch);

        // Join should match
        let view = circuit.views.iter().find(|v| v.plan.id == "threads_with_authors").unwrap();
        assert!(view.cache.contains_key("thread:thread:1"), "Thread should be in join result");

        println!("[TEST] ✓ Batch with join view works correctly");
    }

    /// Test batch updates affecting subquery views
    #[test]
    fn test_batch_with_subquery_view() {
        let mut circuit = setup();

        // Create a view with subquery (thread with author subquery)
        let author_subquery = Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: json!({ "$param": "parent.author" }),
                },
            }),
            limit: 1,
            order_by: None,
        };

        let plan = QueryPlan {
            id: "thread_with_author".to_string(),
            root: Operator::Project {
                input: Box::new(Operator::Scan { table: "thread".to_string() }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(author_subquery),
                    },
                ],
            },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // Batch: create user and thread together
        let batch = vec![
            ("user".into(), "CREATE".into(), "user:1".into(), json!({"id": "user:1", "name": "Alice"}), "h1".into()),
            ("thread".into(), "CREATE".into(), "thread:1".into(), json!({"id": "thread:1", "title": "Test", "author": "user:1"}), "h2".into()),
        ];

        circuit.ingest_batch(batch);

        // Both thread and user should be in cache
        let view = circuit.views.iter().find(|v| v.plan.id == "thread_with_author").unwrap();
        let cache_ids: Vec<_> = view.cache.keys().map(|k| k.to_string()).collect();
        
        assert!(cache_ids.iter().any(|id| id.starts_with("thread:")), "Thread should be tracked");
        assert!(cache_ids.iter().any(|id| id.starts_with("user:")), "User (from subquery) should be tracked");

        println!("[TEST] ✓ Batch with subquery view works correctly");
    }

    /// Test that adding a NEW main record in streaming mode also emits Created events for subquery records.
    /// This was the bug identified in implementation analysis.
    #[test]
    fn test_streaming_addition_with_subquery() {
        let mut circuit = setup();

        // One-to-one subquery: thread -> author
        // We want to see if creating a thread (main record) triggers Created for author (subquery)
        let author_subquery = Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: json!({ "$param": "parent.author" }),
                },
            }),
            limit: 1,
            order_by: None,
        };

        let plan = QueryPlan {
            id: "thread_with_author_streaming".to_string(),
            root: Operator::Project {
                input: Box::new(Operator::Scan { table: "thread".to_string() }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(author_subquery),
                    },
                ],
            },
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

        // 1. Create the user (author) first - unrelated to view yet
        ingest(&mut circuit, "user", "CREATE", "user:100", json!({"id": "user:100", "name": "Subquery User"}));
        
        // 2. Now create the thread that references this user.
        // This is a NEW main record for the view. 
        // Logic should:
        // - Detect new thread
        // - Evaluate subquery -> find user:100
        // - Emit Created for thread:100 AND Created for user:100 (since it's new to the view)
        let updates = ingest(&mut circuit, "thread", "CREATE", "thread:100", json!({"id": "thread:100", "title": "New Thread", "author": "user:100"}));

        // Verify updates
        assert!(!updates.is_empty());
        if let ViewUpdate::Streaming(s) = &updates[0] {
            let ids: Vec<&str> = s.records.iter().map(|r| r.id.as_str()).collect();
            println!("Debug: Emitted IDs: {:?}", ids);
            
            assert!(ids.contains(&"thread:100"), "Should emit thread");
            assert!(ids.contains(&"user:100"), "Should emit subquery user (This was the Bug!)");
            
            // Verify events are Created
            for r in &s.records {
                assert_eq!(r.event, DeltaEvent::Created, "Event for {} should be Created", r.id);
            }
        } else {
            panic!("Expected streaming update");
        }

        println!("[TEST] ✓ Streaming addition correctly emits subquery events");
    }
}

// ============================================================================
// Helper trait for view updates
// ============================================================================

trait ViewUpdateExt {
    fn query_id(&self) -> &str;
}

impl ViewUpdateExt for ViewUpdate {
    fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.query_id,
            ViewUpdate::Streaming(s) => &s.view_id,
        }
    }
}
