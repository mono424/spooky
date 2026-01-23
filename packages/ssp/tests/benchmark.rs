mod common;
use common::*;
use rayon::prelude::*;
use serde_json::json;
use ssp::engine::update::ViewResultFormat;
use ssp::engine::circuit::dto::BatchEntry;
use ssp::engine::types::Operation;
use ssp::{JoinCondition, Operator, Path, Predicate, QueryPlan};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::time::{Duration, Instant};

// --- CONFIGURATION ---
const TOTAL_RECORDS: usize = 10_000;
const VIEW_COUNTS: [usize; 4] = [10, 100, 500, 1000];
const BATCH_SIZE: usize = 100;

struct PreparedRecord {
    table: String,
    op: String,
    id: String,
    record: serde_json::Value,
    hash: String,
}

/// Benchmark comparing Flat vs Streaming format performance
/// Tests the optimizations: Cow<ZSet>, Streaming fast-path, SmolStr VersionMap
#[test]
#[ignore] // Run with: cargo test benchmark_format_comparison --release -- --ignored --nocapture
fn benchmark_format_comparison() {
    let file = File::create("benchmark_format_comparison.csv").expect("Could not create CSV file");
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    writeln!(
        writer,
        "format,views,records,total_time_ms,latency_per_record_ms,ops_per_sec"
    )
    .unwrap();

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║      Stream Processor Performance Benchmark                       ║");
    println!("║      Testing: Cow<ZSet>, Streaming Fast-Path, SmolStr VersionMap ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Configuration: {} records, batch size {}", TOTAL_RECORDS, BATCH_SIZE);
    println!();

    // Test both formats
    for format in [ViewResultFormat::Flat, ViewResultFormat::Streaming] {
        let format_name = match format {
            ViewResultFormat::Flat => "Flat",
            ViewResultFormat::Streaming => "Streaming",
            ViewResultFormat::Tree => "Tree",
        };

        println!("━━━ Testing {} Format ━━━", format_name);

        for &view_count in &VIEW_COUNTS {
            let result = run_benchmark(view_count, format.clone());

            writeln!(
                writer,
                "{},{},{},{:.2},{:.4},{:.2}",
                format_name,
                view_count,
                TOTAL_RECORDS,
                result.total_time_ms,
                result.latency_per_record_ms,
                result.ops_per_sec
            )
            .unwrap();

            println!(
                "  {} views: {:.1}ms total | {:.3}ms/record | {:.0} ops/sec",
                view_count, result.total_time_ms, result.latency_per_record_ms, result.ops_per_sec
            );
        }
        println!();
    }

    writer.flush().unwrap();
    println!("Results saved to benchmark_format_comparison.csv");
}

/// Quick benchmark for streaming mode only (faster feedback loop)
#[test]
#[ignore] // Run with: cargo test benchmark_streaming_quick --release -- --ignored --nocapture
fn benchmark_streaming_quick() {
    println!("╔════════════════════════════════════════╗");
    println!("║  Quick Streaming Mode Benchmark        ║");
    println!("╚════════════════════════════════════════╝\n");

    for &view_count in &[10, 100, 500] {
        let result = run_benchmark(view_count, ViewResultFormat::Streaming);
        println!(
            "{:4} views: {:7.1}ms | {:6.3}ms/rec | {:8.0} ops/sec",
            view_count, result.total_time_ms, result.latency_per_record_ms, result.ops_per_sec
        );
    }
}

/// Original comprehensive benchmark (Flat mode, all view counts)
#[test]
#[ignore] // Run with: cargo test benchmark_latency_mixed_stream --release -- --ignored --nocapture
fn benchmark_latency_mixed_stream() {
    let file = File::create("benchmark_results.csv").expect("Could not create CSV file");
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    writeln!(
        writer,
        "views,records,total_time_ms,latency_last_batch_ms,ops_per_sec"
    )
    .unwrap();

    println!(
        "Start Benchmark ({} Records Total, Batch Size {})...",
        TOTAL_RECORDS, BATCH_SIZE
    );

    for &view_count in &VIEW_COUNTS {
        let mut circuit = setup();

        // --- 1. Views Setup (Flat mode - original behavior) ---
        print!(">> Setup {} Views... ", view_count);
        io::stdout().flush().unwrap();
        for i in 0..view_count {
            let plan = create_magic_comments_plan(&format!("view_{}", i));
            circuit.register_view(plan, None, None); // None = Flat format (default)
        }
        println!("Done.");

        // --- 2. Prepare Data (Parallel with Rayon) ---
        let prepared_stream = prepare_test_data();

        // --- 3. Ingest Loop & Measurement ---
        let mut total_ingest_duration = Duration::new(0, 0);
        let mut global_record_count = 0;

        for chunk in prepared_stream.chunks(BATCH_SIZE) {
            let batch_data: Vec<BatchEntry> = chunk
                .iter()
                .map(|item| {
                    BatchEntry::new(
                        item.table.clone(),
                        Operation::from_str(&item.op).unwrap(),
                        item.id.clone(),
                        item.record.clone().into(),
                    )
                })
                .collect();

            let batch_len = batch_data.len();

            let start = Instant::now();
            circuit.ingest_batch(batch_data);
            let duration = start.elapsed();

            total_ingest_duration += duration;
            global_record_count += batch_len;

            let total_ms = total_ingest_duration.as_secs_f64() * 1000.0;
            let latency_last_batch_ms = (duration.as_secs_f64() * 1000.0) / batch_len as f64;
            let ops_sec = global_record_count as f64 / total_ingest_duration.as_secs_f64();

            writeln!(
                writer,
                "{},{},{:.2},{:.4},{:.2}",
                view_count, global_record_count, total_ms, latency_last_batch_ms, ops_sec
            )
            .unwrap();

            print!(
                "\r>> Views: {} | Records: {}/{} | Latency: {:.3} ms | Speed: {:.0} ops/sec",
                view_count, global_record_count, TOTAL_RECORDS, latency_last_batch_ms, ops_sec
            );
            io::stdout().flush().unwrap();
        }

        println!();
    }
    println!("Benchmark complete. Results in 'benchmark_results.csv'.");
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

struct BenchmarkResult {
    total_time_ms: f64,
    latency_per_record_ms: f64,
    ops_per_sec: f64,
}

fn run_benchmark(view_count: usize, format: ViewResultFormat) -> BenchmarkResult {
    let mut circuit = setup();

    // Register views with specified format
    for i in 0..view_count {
        let plan = create_magic_comments_plan(&format!("view_{}", i));
        circuit.register_view(plan, None, Some(format.clone()));
    }

    // Prepare data
    let prepared_stream = prepare_test_data();

    // Ingest and measure
    let start = Instant::now();
    for chunk in prepared_stream.chunks(BATCH_SIZE) {
        let batch_data: Vec<BatchEntry> = chunk
            .iter()
            .map(|item| {
                BatchEntry::new(
                    item.table.clone(),
                    Operation::from_str(&item.op).unwrap(),
                    item.id.clone(),
                    item.record.clone().into(),
                )
            })
            .collect();
        circuit.ingest_batch(batch_data);
    }
    let total_duration = start.elapsed();

    let total_time_ms = total_duration.as_secs_f64() * 1000.0;
    let latency_per_record_ms = total_time_ms / TOTAL_RECORDS as f64;
    let ops_per_sec = TOTAL_RECORDS as f64 / total_duration.as_secs_f64();

    BenchmarkResult {
        total_time_ms,
        latency_per_record_ms,
        ops_per_sec,
    }
}

fn prepare_test_data() -> Vec<PreparedRecord> {
    let sets_needed = (TOTAL_RECORDS as f64 / 3.0).ceil() as usize;

    let mut prepared_stream: Vec<PreparedRecord> = (0..sets_needed)
        .into_par_iter()
        .map(|_| {
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

            // 2. Thread
            let (thread_id, rec_thread) = make_thread_record("BenchThread", &author_id);
            batch.push(PreparedRecord {
                table: "thread".to_string(),
                op: "CREATE".to_string(),
                id: thread_id.clone(),
                hash: generate_hash(&rec_thread),
                record: rec_thread,
            });

            // 3. Comment
            let (comment_id, rec_comment) = make_comment_record("Magic", &thread_id, &author_id);
            batch.push(PreparedRecord {
                table: "comment".to_string(),
                op: "CREATE".to_string(),
                id: comment_id.clone(),
                hash: generate_hash(&rec_comment),
                record: rec_comment,
            });

            batch
        })
        .flatten()
        .collect();

    prepared_stream.truncate(TOTAL_RECORDS);
    prepared_stream
}

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
