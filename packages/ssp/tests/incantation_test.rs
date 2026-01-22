use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use ssp::{Circuit, JoinCondition, Operator, Path, Predicate, QueryPlan};

#[test]
fn test_complex_incantation_flow() {
    let mut circuit = setup();

    // 1. Setup Base Data
    let author_alice = create_author(&mut circuit, "Alice");

    // Thread 1 by Alice
    let thread_1 = create_thread(&mut circuit, "Thread 1", &author_alice);

    // 2. Define Query Plan
    // Goal: Find Threads by Alice that have comments with text "Magic"

    // Scan Threads
    let scan_threads = Operator::Scan {
        table: "thread".to_string(),
    };

    // Scan Authors
    let scan_authors = Operator::Scan {
        table: "author".to_string(),
    };

    // Join Threads with Authors (Ensure Author Exists)
    let threads_with_authors = Operator::Join {
        left: Box::new(scan_threads),
        right: Box::new(scan_authors),
        on: JoinCondition {
            left_field: Path::new("author"),
            right_field: Path::new("id"),
        },
    };

    // Scan Comments
    let scan_comments = Operator::Scan {
        table: "comment".to_string(),
    };

    // Filter Comments for "Magic"
    let magic_comments = Operator::Filter {
        input: Box::new(scan_comments),
        predicate: Predicate::Eq {
            field: Path::new("text"),
            value: json!("Magic"),
        },
    };

    // Join (Threads+Authors) with MagicComments
    // This effectively filters threads to only those having at least one magic comment
    let root = Operator::Join {
        left: Box::new(threads_with_authors),
        right: Box::new(magic_comments),
        on: JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("thread"),
        },
    };

    let plan = QueryPlan {
        id: "magic_threads_by_alice".to_string(),
        root,
    };

    // 3. Register View
    let initial_update = circuit.register_view(plan, None, None);

    // Initially, Thread 1 exists and Author exists, but no comments.
    // So result should be empty.
    if let Some(up) = initial_update {
        assert!(up.result_data().is_empty(), "Expected empty result initially");
    }

    // 4. Verify View State Helper
    let check_view = |circuit: &Circuit, expect_present: bool| {
        let view = circuit
            .views
            .iter()
            .find(|v| v.plan.id == "magic_threads_by_alice")
            .expect("View not found");
        // circuit stores keys as "table:id", and thread_1 is "thread:xxx". 
        // ingest prefixes it during storage. View cache holds storage keys.
        let storage_key = format!("thread:{}", thread_1);
        let present = view.cache.contains_key(storage_key.as_str());
        assert_eq!(present, expect_present, "Thread 1 presence mismatch in cache");
    };

    check_view(&circuit, false);

    // 5. Add "Boring" Comment -> Should NOT trigger view
    let _boring_comment = create_comment(&mut circuit, "Boring", &thread_1, &author_alice);
    check_view(&circuit, false);

    // 6. Add "Magic" Comment -> Should trigger view (Thread 1 Appears)
    let magic_comment_id = create_comment(&mut circuit, "Magic", &thread_1, &author_alice);
    check_view(&circuit, true);

    // 7. Add another "Magic" Comment -> Thread 1 still present
    let magic_comment_2 = create_comment(&mut circuit, "Magic", &thread_1, &author_alice);
    check_view(&circuit, true);

    // 8. Delete the first Magic Comment -> Thread 1 still present (count > 0)
    ingest(
        &mut circuit,
        "comment",
        "DELETE",
        &magic_comment_id,
        json!({}),

    );
    check_view(&circuit, true);

    // 9. Delete the second Magic Comment -> Thread 1 disappears
    ingest(
        &mut circuit,
        "comment",
        "DELETE",
        &magic_comment_2,
        json!({}),

    );
    check_view(&circuit, false);

    // 10. Delete Author -> Thread 1 disappears (dependency check currently empty but good to test)
    // Note: It's already gone, but let's re-add a magic comment to verify dependency deletion works
    let _magic_comment_3 = create_comment(&mut circuit, "Magic", &thread_1, &author_alice);
    check_view(&circuit, true);

    ingest(&mut circuit, "author", "DELETE", &author_alice, json!({}));
    check_view(&circuit, false);
}
