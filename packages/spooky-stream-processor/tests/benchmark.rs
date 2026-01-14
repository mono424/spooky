mod common;
use common::*;
use rayon::prelude::*;
use serde_json::json;
use spooky_stream_processor::engine::view::{JoinCondition, Operator, Path, Predicate, QueryPlan};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::time::{Duration, Instant};

// --- CONFIGURATION ---
const TOTAL_RECORDS: usize = 10_000;
const VIEW_COUNTS: [usize; 4] = [10, 100, 500, 1000];
const BATCH_SIZE: usize = 100;
const WARMUP_BATCHES: usize = 3;

struct PreparedRecord {
    table: String,
    op: String,
    id: String,
    record: serde_json::Value,
    hash: String,
}

/// Generate a linked set of records: Author -> Thread -> Comment
fn make_linked_record_set() -> Vec<PreparedRecord> {
    let mut batch = Vec::with_capacity(3);

    // 1. Author
    let (author_id, rec_auth) = make_author_record("BenchUser");
    batch.push(PreparedRecord {
        table: "author".to_string(),
        op: "CREATE".to_string(),
        id: author_id.clone(),
        hash: generate_hash(&rec_auth),
        record: rec_auth,
    });

    // 2. Thread (links to author)
    let (thread_id, rec_thread) = make_thread_record("BenchThread", &author_id);
    batch.push(PreparedRecord {
        table: "thread".to_string(),
        op: "CREATE".to_string(),
        id: thread_id.clone(),
        hash: generate_hash(&rec_thread),
        record: rec_thread,
    });

    // 3. Comment (links to thread and author)
    let (comment_id, rec_comment) = make_comment_record("Magic", &thread_id, &author_id);
    batch.push(PreparedRecord {
        table: "comment".to_string(),
        op: "CREATE".to_string(),
        id: comment_id,
        hash: generate_hash(&rec_comment),
        record: rec_comment,
    });

    batch
}

/// Convert PreparedRecord to batch tuple format
fn to_batch_tuple(item: &PreparedRecord) -> (String, String, String, serde_json::Value, String) {
    (
        item.table.clone(),
        item.op.clone(),
        item.id.clone(),
        item.record.clone(),
        item.hash.clone(),
    )
}

#[test]
#[ignore] // Run with: cargo test --release benchmark_latency_mixed_stream -- --nocapture --ignored
fn benchmark_latency_mixed_stream() {
    let file = File::create("benchmark_results.csv").expect("Could not create CSV file");
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    writeln!(
        writer,
        "views,records,phase,total_time_ms,latency_per_record_ms,ops_per_sec"
    )
    .unwrap();

    println!(
        "=== BENCHMARK: {} Records, Batch Size {} ===\n",
        TOTAL_RECORDS, BATCH_SIZE
    );

    for &view_count in &VIEW_COUNTS {
        let mut circuit = setup();

        // --- 1. VIEWS SETUP ---
        print!("> Setting up {} views... ", view_count);
        io::stdout().flush().unwrap();
        for i in 0..view_count {
            let plan = create_magic_comments_plan(&format!("view_{}", i));
            circuit.register_view(plan, None);
        }
        println!("done.");

        // --- 2. DATA PREPARATION (parallel, outside measurement) ---
        let sets_needed = (TOTAL_RECORDS as f64 / 3.0).ceil() as usize;

        let prepared_stream: Vec<PreparedRecord> = (0..sets_needed)
            .into_par_iter()
            .flat_map(|_| make_linked_record_set())
            .collect();

        // Prepare all batch data BEFORE measurement (avoid clone overhead in hot path)
        let all_batches: Vec<Vec<(String, String, String, serde_json::Value, String)>> = prepared_stream
            .chunks(BATCH_SIZE)
            .take(TOTAL_RECORDS / BATCH_SIZE)
            .map(|chunk| chunk.iter().map(to_batch_tuple).collect())
            .collect();

        // --- 3. WARMUP (not measured) ---
        print!("> Warmup ({} batches)... ", WARMUP_BATCHES);
        io::stdout().flush().unwrap();
        for batch in all_batches.iter().take(WARMUP_BATCHES) {
            circuit.ingest_batch(batch.clone());
        }
        println!("done.");

        // --- 4. CREATE PHASE (measure INSERT throughput) ---
        let mut total_duration = Duration::ZERO;
        let mut record_count = 0;

        print!("> CREATE phase: ");
        io::stdout().flush().unwrap();

        for batch in all_batches.iter().skip(WARMUP_BATCHES) {
            let batch_len = batch.len();

            // === MEASUREMENT START ===
            let start = Instant::now();
            circuit.ingest_batch(batch.clone());
            let duration = start.elapsed();
            // === MEASUREMENT END ===

            total_duration += duration;
            record_count += batch_len;

            let latency_ms = (duration.as_secs_f64() * 1000.0) / batch_len as f64;
            let ops_sec = record_count as f64 / total_duration.as_secs_f64();

            writeln!(
                writer,
                "{},{},CREATE,{:.2},{:.4},{:.2}",
                view_count,
                record_count,
                total_duration.as_secs_f64() * 1000.0,
                latency_ms,
                ops_sec
            )
            .unwrap();

            print!("\r> CREATE phase: {}/{} | {:.3} ms/rec | {:.0} ops/sec   ",
                record_count, TOTAL_RECORDS - (WARMUP_BATCHES * BATCH_SIZE), latency_ms, ops_sec);
            io::stdout().flush().unwrap();
        }
        println!();

        // --- 5. UPDATE PHASE (measure incremental delta throughput) ---
        // Create UPDATE records for existing IDs to test O(Delta) path
        let update_count = record_count.min(1000); // Update up to 1000 existing records
        let existing_ids: Vec<&PreparedRecord> = prepared_stream
            .iter()
            .filter(|r| r.table == "comment")
            .take(update_count)
            .collect();

        let update_batches: Vec<Vec<(String, String, String, serde_json::Value, String)>> = existing_ids
            .chunks(BATCH_SIZE)
            .map(|chunk| {
                chunk.iter().map(|item| {
                    // Create UPDATE with modified text
                    let updated_record = json!({
                        "id": item.id,
                        "text": "UpdatedMagic",
                        "thread": item.record.get("thread").unwrap(),
                        "author": item.record.get("author").unwrap(),
                        "type": "comment"
                    });
                    (
                        item.table.clone(),
                        "UPDATE".to_string(),
                        item.id.clone(),
                        updated_record.clone(),
                        generate_hash(&updated_record),
                    )
                }).collect()
            })
            .collect();

        let mut update_duration = Duration::ZERO;
        let mut update_record_count = 0;

        print!("> UPDATE phase: ");
        io::stdout().flush().unwrap();

        for batch in update_batches.iter() {
            let batch_len = batch.len();

            let start = Instant::now();
            circuit.ingest_batch(batch.clone());
            let duration = start.elapsed();

            update_duration += duration;
            update_record_count += batch_len;

            let latency_ms = (duration.as_secs_f64() * 1000.0) / batch_len as f64;
            let ops_sec = update_record_count as f64 / update_duration.as_secs_f64();

            writeln!(
                writer,
                "{},{},UPDATE,{:.2},{:.4},{:.2}",
                view_count,
                update_record_count,
                update_duration.as_secs_f64() * 1000.0,
                latency_ms,
                ops_sec
            )
            .unwrap();

            print!("\r> UPDATE phase: {}/{} | {:.3} ms/rec | {:.0} ops/sec   ",
                update_record_count, update_count, latency_ms, ops_sec);
            io::stdout().flush().unwrap();
        }
        println!();

        // --- 6. VERIFICATION ---
        let view = circuit.views.iter().find(|v| v.plan.id == "view_0").unwrap();
        let cache_size = view.cache.len();
        println!("> Verification: view_0 cache has {} entries\n", cache_size);
    }

    println!("=== Benchmark complete. Results in 'benchmark_results.csv' ===");
}

/// Creates a query plan: threads WITH authors JOIN comments WHERE text = "Magic"
fn create_magic_comments_plan(view_id: &str) -> QueryPlan {
    let scan_threads = Operator::Scan {
        table: "thread".to_string(),
    };
    let scan_authors = Operator::Scan {
        table: "author".to_string(),
    };

    let threads_with_authors = Operator::Join {
        left: Box::new(scan_threads),
        right: Box::new(scan_authors),
        on: JoinCondition {
            left_field: Path::new("author"),
            right_field: Path::new("id"),
        },
    };

    let scan_comments = Operator::Scan {
        table: "comment".to_string(),
    };
    let magic_comments = Operator::Filter {
        input: Box::new(scan_comments),
        predicate: Predicate::Eq {
            field: Path::new("text"),
            value: json!("Magic"),
        },
    };

    let root = Operator::Join {
        left: Box::new(threads_with_authors),
        right: Box::new(magic_comments),
        on: JoinCondition {
            left_field: Path::new("id"),
            right_field: Path::new("thread"),
        },
    };

    QueryPlan {
        id: view_id.to_string(),
        root,
    }
}
