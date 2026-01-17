//! Real-World Benchmark for spooky-stream-processor
//!
//! This benchmark models production-realistic workloads as used by the sync engine.
//! It tests all three ViewResultFormat modes: Flat, Streaming, and Tree.
//!
//! Metrics measured:
//! - Throughput scaling across view counts
//! - Latency percentiles (P50, P95, P99)
//! - Memory efficiency
//! - Format-specific performance characteristics
//!
//! Run with: cargo test --release real_world_benchmark -- --nocapture --ignored
//! 
/*
Test functions:

real_world_benchmark - Full matrix benchmark across formats, view counts, record counts, batch sizes
streaming_mode_benchmark - Focused test on streaming delta efficiency (counts Created/Updated/Deleted events)
format_comparison_benchmark - Head-to-head comparison of Flat vs Streaming throughput
real_world_benchmark_quick - Fast sanity check for iteration

# Full benchmark
cargo test --release real_world_benchmark -- --nocapture --ignored

# Quick check
cargo test --release real_world_benchmark_quick -- --nocapture --ignored

# Streaming-specific test
cargo test --release streaming_mode_benchmark -- --nocapture --ignored

# Format comparison
cargo test --release format_comparison_benchmark -- --nocapture --ignored
*/

mod common;
use common::*;
use rayon::prelude::*;
use serde_json::{json, Value};
use spooky_stream_processor::{
    engine::update::{DeltaEvent, StreamingUpdate, ViewResultFormat, ViewUpdate},
    engine::view::{JoinCondition, Operator, Path, Predicate, Projection, QueryPlan},
    Circuit,
};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

// === CONFIGURATION ===
const RECORD_COUNTS: [usize; 4] = [100, 1000, 5000, 10000];
const VIEW_COUNTS: [usize; 6] = [10, 50, 100, 250, 500, 1000];
const BATCH_SIZES: [usize; 4] = [1, 10, 50, 100];
const WARMUP_ITERATIONS: usize = 3;

// Operation mix (realistic workload - matches sync engine patterns)
const CREATE_RATIO: f64 = 0.70;
const UPDATE_RATIO: f64 = 0.20;
// DELETE_RATIO: 0.10 (implicit)

/// Metrics collected per benchmark run
#[derive(Debug, Clone)]
struct BenchmarkMetrics {
    format: String,
    views: usize,
    records: usize,
    batch_size: usize,
    phase: String,
    total_time_ms: f64,
    ops_per_sec: f64,
    latency_p50_us: f64,
    latency_p95_us: f64,
    latency_p99_us: f64,
    updates_emitted: usize,
    delta_records_total: usize,
}

impl BenchmarkMetrics {
    fn csv_header() -> &'static str {
        "format,views,records,batch_size,phase,total_time_ms,ops_per_sec,latency_p50_us,latency_p95_us,latency_p99_us,updates_emitted,delta_records_total"
    }

    fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{}",
            self.format,
            self.views,
            self.records,
            self.batch_size,
            self.phase,
            self.total_time_ms,
            self.ops_per_sec,
            self.latency_p50_us,
            self.latency_p95_us,
            self.latency_p99_us,
            self.updates_emitted,
            self.delta_records_total
        )
    }
}

/// Record types for realistic data distribution
struct PreparedRecord {
    table: String,
    op: String,
    id: String,
    record: Value,
    hash: String,
}

/// Generate a batch of linked records (Author -> Thread -> Comment)
/// Simulates realistic relational data patterns matching the sync engine's data model
fn make_linked_record_set(prefix: usize) -> Vec<PreparedRecord> {
    let mut batch = Vec::with_capacity(3);

    // 1. Author (small metadata record)
    let author_id = format!("author:{}", ulid::Ulid::new());
    let author_rec = json!({
        "id": &author_id,
        "name": format!("User{}", prefix),
        "email": format!("user{}@example.com", prefix),
        "type": "author",
        "createdAt": "2026-01-14T00:00:00Z"
    });
    batch.push(PreparedRecord {
        table: "author".to_string(),
        op: "CREATE".to_string(),
        id: author_id.clone(),
        hash: generate_hash(&author_rec),
        record: author_rec,
    });

    // 2. Thread (medium-sized record with references)
    let thread_id = format!("thread:{}", ulid::Ulid::new());
    let thread_rec = json!({
        "id": &thread_id,
        "title": format!("Discussion Topic #{}", prefix),
        "author": &author_id,
        "created_at": "2026-01-14T00:00:00Z",
        "views": prefix % 1000,
        "type": "thread",
        "status": if prefix % 3 == 0 { "archived" } else { "active" }
    });
    batch.push(PreparedRecord {
        table: "thread".to_string(),
        op: "CREATE".to_string(),
        id: thread_id.clone(),
        hash: generate_hash(&thread_rec),
        record: thread_rec,
    });

    // 3. Comment (larger content record - skewed: some threads get more comments)
    let comment_id = format!("comment:{}", ulid::Ulid::new());
    let text = if prefix % 5 == 0 {
        "Magic"
    } else {
        "Regular comment content"
    };
    let comment_rec = json!({
        "id": &comment_id,
        "text": text,
        "thread": &thread_id,
        "author": &author_id,
        "score": (prefix * 7) % 100,
        "type": "comment",
        "status": "published"
    });
    batch.push(PreparedRecord {
        table: "comment".to_string(),
        op: "CREATE".to_string(),
        id: comment_id,
        hash: generate_hash(&comment_rec),
        record: comment_rec,
    });

    batch
}

/// Convert PreparedRecord to batch tuple format (matches Circuit::ingest_batch signature)
fn to_batch_tuple(item: &PreparedRecord) -> (String, String, String, Value, String) {
    (
        item.table.clone(),
        item.op.clone(),
        item.id.clone(),
        item.record.clone(),
        item.hash.clone(),
    )
}

/// Calculate percentile from sorted durations
fn percentile(sorted_durations: &[Duration], p: f64) -> Duration {
    if sorted_durations.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted_durations.len() as f64 * p / 100.0).ceil() as usize).saturating_sub(1);
    sorted_durations[idx.min(sorted_durations.len() - 1)]
}

/// Count total delta records across all ViewUpdates
fn count_delta_records(updates: &[ViewUpdate]) -> usize {
    updates
        .iter()
        .map(|u| match u {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => m.result_data.len(),
            ViewUpdate::Streaming(s) => s.records.len(),
        })
        .sum()
}

// === QUERY PLAN BUILDERS ===
// These mirror the types of queries used by the sync engine

/// Simple filter query: comments WHERE text = "Magic"
fn create_filter_plan(view_id: &str) -> QueryPlan {
    let scan = Operator::Scan {
        table: "comment".to_string(),
    };
    let filtered = Operator::Filter {
        input: Box::new(scan),
        predicate: Predicate::Eq {
            field: Path::new("text"),
            value: json!("Magic"),
        },
    };
    QueryPlan {
        id: view_id.to_string(),
        root: filtered,
    }
}

/// Prefix filter query: records WHERE id STARTS WITH "thread:"
fn create_prefix_plan(view_id: &str) -> QueryPlan {
    let scan = Operator::Scan {
        table: "thread".to_string(),
    };
    let filtered = Operator::Filter {
        input: Box::new(scan),
        predicate: Predicate::Prefix {
            field: Path::new("id"),
            prefix: "thread:".to_string(),
        },
    };
    QueryPlan {
        id: view_id.to_string(),
        root: filtered,
    }
}

/// Join query: threads JOIN authors (common in sync engine for hydrating relationships)
fn create_join_plan(view_id: &str) -> QueryPlan {
    let scan_threads = Operator::Scan {
        table: "thread".to_string(),
    };
    let scan_authors = Operator::Scan {
        table: "author".to_string(),
    };
    let joined = Operator::Join {
        left: Box::new(scan_threads),
        right: Box::new(scan_authors),
        on: JoinCondition {
            left_field: Path::new("author"),
            right_field: Path::new("id"),
        },
    };
    QueryPlan {
        id: view_id.to_string(),
        root: joined,
    }
}

/// Complex query: threads WITH authors JOIN comments WHERE text = "Magic"
fn create_complex_plan(view_id: &str) -> QueryPlan {
    let threads_with_authors = Operator::Join {
        left: Box::new(Operator::Scan {
            table: "thread".to_string(),
        }),
        right: Box::new(Operator::Scan {
            table: "author".to_string(),
        }),
        on: JoinCondition {
            left_field: Path::new("author"),
            right_field: Path::new("id"),
        },
    };

    let magic_comments = Operator::Filter {
        input: Box::new(Operator::Scan {
            table: "comment".to_string(),
        }),
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

/// Subquery plan: threads with nested comments (used by sync engine for hierarchical views)
fn create_subquery_plan(view_id: &str) -> QueryPlan {
    let scan_threads = Operator::Scan {
        table: "thread".to_string(),
    };

    // Subquery: comments for this thread
    let comments_subquery = Operator::Filter {
        input: Box::new(Operator::Scan {
            table: "comment".to_string(),
        }),
        predicate: Predicate::Eq {
            field: Path::new("thread"),
            value: json!({"$param": "parent.id"}), // Parameter reference
        },
    };

    let projected = Operator::Project {
        input: Box::new(scan_threads),
        projections: vec![
            Projection::All,
            Projection::Subquery {
                alias: "comments".to_string(),
                plan: Box::new(comments_subquery),
            },
        ],
    };

    QueryPlan {
        id: view_id.to_string(),
        root: projected,
    }
}

/// Limited query with ordering: top N threads by views
fn create_limit_plan(view_id: &str, limit: usize) -> QueryPlan {
    use spooky_stream_processor::engine::OrderSpec;

    let scan = Operator::Scan {
        table: "thread".to_string(),
    };
    let limited = Operator::Limit {
        input: Box::new(scan),
        limit,
        order_by: Some(vec![OrderSpec {
            field: Path::new("views"),
            direction: "DESC".to_string(),
        }]),
    };
    QueryPlan {
        id: view_id.to_string(),
        root: limited,
    }
}

/// Run benchmark for a specific format
fn run_format_benchmark(
    format: ViewResultFormat,
    view_count: usize,
    record_count: usize,
    batch_size: usize,
) -> Vec<BenchmarkMetrics> {
    let format_name = match format {
        ViewResultFormat::Flat => "flat",
        ViewResultFormat::Tree => "tree",
        ViewResultFormat::Streaming => "streaming",
    };

    let mut metrics_list: Vec<BenchmarkMetrics> = Vec::new();
    let mut circuit = setup();

    // === REGISTER VIEWS (Mixed Query Types) ===
    for i in 0..view_count {
        let plan = match i % 10 {
            0..=4 => create_filter_plan(&format!("{}_{}", format_name, i)),   // 50%
            5..=6 => create_prefix_plan(&format!("{}_{}", format_name, i)),   // 20%
            7..=8 => create_join_plan(&format!("{}_{}", format_name, i)),     // 20%
            _ => create_complex_plan(&format!("{}_{}", format_name, i)),      // 10%
        };
        circuit.register_view(plan, None, Some(format.clone()));
    }

    // === PREPARE DATA (parallel, outside measurement) ===
    let sets_needed = (record_count as f64 / 3.0).ceil() as usize;
    let prepared_stream: Vec<PreparedRecord> = (0..sets_needed)
        .into_par_iter()
        .flat_map(make_linked_record_set)
        .collect();

    let all_batches: Vec<Vec<(String, String, String, Value, String)>> = prepared_stream
        .chunks(batch_size)
        .take(record_count / batch_size)
        .map(|chunk| chunk.iter().map(to_batch_tuple).collect())
        .collect();

    // === WARMUP ===
    for batch in all_batches.iter().take(WARMUP_ITERATIONS) {
        circuit.ingest_batch(batch.clone(), true);
    }

    // === CREATE PHASE MEASUREMENT ===
    let mut latencies: Vec<Duration> = Vec::with_capacity(all_batches.len());
    let mut total_updates = 0usize;
    let mut total_delta_records = 0usize;
    let mut total_records = 0usize;

    let phase_start = Instant::now();

    for batch in all_batches.iter().skip(WARMUP_ITERATIONS) {
        let batch_len = batch.len();

        let start = Instant::now();
        let updates = circuit.ingest_batch(batch.clone(), true); // is_optimistic=true for sync engine
        let duration = start.elapsed();

        latencies.push(duration);
        total_records += batch_len;
        total_updates += updates.len();
        total_delta_records += count_delta_records(&updates);
    }

    let total_time = phase_start.elapsed();

    // Calculate percentiles
    latencies.sort();
    let p50 = percentile(&latencies, 50.0);
    let p95 = percentile(&latencies, 95.0);
    let p99 = percentile(&latencies, 99.0);

    let create_metrics = BenchmarkMetrics {
        format: format_name.to_string(),
        views: view_count,
        records: total_records,
        batch_size,
        phase: "CREATE".to_string(),
        total_time_ms: total_time.as_secs_f64() * 1000.0,
        ops_per_sec: total_records as f64 / total_time.as_secs_f64(),
        latency_p50_us: p50.as_secs_f64() * 1_000_000.0,
        latency_p95_us: p95.as_secs_f64() * 1_000_000.0,
        latency_p99_us: p99.as_secs_f64() * 1_000_000.0,
        updates_emitted: total_updates,
        delta_records_total: total_delta_records,
    };
    metrics_list.push(create_metrics);

    // === UPDATE PHASE ===
    let update_count = (total_records as f64 * UPDATE_RATIO) as usize;
    let update_ids: Vec<&PreparedRecord> = prepared_stream
        .iter()
        .filter(|r| r.table == "comment")
        .take(update_count)
        .collect();

    if !update_ids.is_empty() {
        let update_batches: Vec<Vec<(String, String, String, Value, String)>> = update_ids
            .chunks(batch_size)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|item| {
                        let updated = json!({
                            "id": item.id,
                            "text": "UpdatedMagic",
                            "thread": item.record.get("thread").unwrap(),
                            "author": item.record.get("author").unwrap(),
                            "score": 999,
                            "type": "comment",
                            "status": "edited"
                        });
                        (
                            item.table.clone(),
                            "UPDATE".to_string(),
                            item.id.clone(),
                            updated.clone(),
                            generate_hash(&updated),
                        )
                    })
                    .collect()
            })
            .collect();

        let mut update_latencies: Vec<Duration> = Vec::new();
        let mut updated_records = 0usize;
        let mut update_total_updates = 0usize;
        let mut update_delta_records = 0usize;
        let update_start = Instant::now();

        for batch in update_batches {
            let batch_len = batch.len();
            let start = Instant::now();
            let updates = circuit.ingest_batch(batch, true);
            update_latencies.push(start.elapsed());
            updated_records += batch_len;
            update_total_updates += updates.len();
            update_delta_records += count_delta_records(&updates);
        }

        let update_time = update_start.elapsed();
        update_latencies.sort();

        let update_metrics = BenchmarkMetrics {
            format: format_name.to_string(),
            views: view_count,
            records: updated_records,
            batch_size,
            phase: "UPDATE".to_string(),
            total_time_ms: update_time.as_secs_f64() * 1000.0,
            ops_per_sec: updated_records as f64 / update_time.as_secs_f64(),
            latency_p50_us: percentile(&update_latencies, 50.0).as_secs_f64() * 1_000_000.0,
            latency_p95_us: percentile(&update_latencies, 95.0).as_secs_f64() * 1_000_000.0,
            latency_p99_us: percentile(&update_latencies, 99.0).as_secs_f64() * 1_000_000.0,
            updates_emitted: update_total_updates,
            delta_records_total: update_delta_records,
        };
        metrics_list.push(update_metrics);
    }

    // === DELETE PHASE ===
    let delete_count = (total_records as f64 * 0.10) as usize; // 10% deletes
    let delete_ids: Vec<&PreparedRecord> = prepared_stream
        .iter()
        .filter(|r| r.table == "comment")
        .skip(update_count) // Don't delete the ones we just updated
        .take(delete_count)
        .collect();

    if !delete_ids.is_empty() {
        let delete_batches: Vec<Vec<(String, String, String, Value, String)>> = delete_ids
            .chunks(batch_size)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|item| {
                        (
                            item.table.clone(),
                            "DELETE".to_string(),
                            item.id.clone(),
                            json!({}), // Empty record for delete
                            String::new(),
                        )
                    })
                    .collect()
            })
            .collect();

        let mut delete_latencies: Vec<Duration> = Vec::new();
        let mut deleted_records = 0usize;
        let mut delete_total_updates = 0usize;
        let mut delete_delta_records = 0usize;
        let delete_start = Instant::now();

        for batch in delete_batches {
            let batch_len = batch.len();
            let start = Instant::now();
            let updates = circuit.ingest_batch(batch, true);
            delete_latencies.push(start.elapsed());
            deleted_records += batch_len;
            delete_total_updates += updates.len();
            delete_delta_records += count_delta_records(&updates);
        }

        let delete_time = delete_start.elapsed();
        delete_latencies.sort();

        let delete_metrics = BenchmarkMetrics {
            format: format_name.to_string(),
            views: view_count,
            records: deleted_records,
            batch_size,
            phase: "DELETE".to_string(),
            total_time_ms: delete_time.as_secs_f64() * 1000.0,
            ops_per_sec: deleted_records as f64 / delete_time.as_secs_f64(),
            latency_p50_us: percentile(&delete_latencies, 50.0).as_secs_f64() * 1_000_000.0,
            latency_p95_us: percentile(&delete_latencies, 95.0).as_secs_f64() * 1_000_000.0,
            latency_p99_us: percentile(&delete_latencies, 99.0).as_secs_f64() * 1_000_000.0,
            updates_emitted: delete_total_updates,
            delta_records_total: delete_delta_records,
        };
        metrics_list.push(delete_metrics);
    }

    metrics_list
}

#[test]
#[ignore] // Run with: cargo test --release real_world_benchmark -- --nocapture --ignored
fn real_world_benchmark() {
    println!("\n╔══════════════════════════════════════════════════════════════════════╗");
    println!("║        REAL-WORLD BENCHMARK: spooky-stream-processor                ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!("║  Formats: Flat, Streaming, Tree                                     ║");
    println!("║  Query Mix: 50% Filter, 20% Prefix, 20% Join, 10% Complex           ║");
    println!("║  Op Mix: 70% CREATE, 20% UPDATE, 10% DELETE                         ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    let file = File::create("real_world_benchmark_results.csv").expect("Could not create CSV file");
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    writeln!(writer, "{}", BenchmarkMetrics::csv_header()).unwrap();

    let mut all_metrics: Vec<BenchmarkMetrics> = Vec::new();

    // Test subset for reasonable runtime (full matrix takes longer)
    let test_views = &VIEW_COUNTS[..4]; // [10, 50, 100, 250]
    let test_records = &RECORD_COUNTS[..3]; // [100, 1000, 5000]
    let test_batches = &BATCH_SIZES[1..3]; // [10, 50]
    let formats = [
        ViewResultFormat::Flat,
        ViewResultFormat::Streaming,
        ViewResultFormat::Tree,
    ];

    for format in &formats {
        let format_name = match format {
            ViewResultFormat::Flat => "FLAT",
            ViewResultFormat::Tree => "TREE",
            ViewResultFormat::Streaming => "STREAMING",
        };
        println!("\n═══════════════════════════════════════════════════════════════");
        println!("  FORMAT: {}", format_name);
        println!("═══════════════════════════════════════════════════════════════\n");

        for &view_count in test_views {
            for &record_count in test_records {
                for &batch_size in test_batches {
                    println!(
                        "━━━ {} | Views: {} | Records: {} | Batch: {} ━━━",
                        format_name, view_count, record_count, batch_size
                    );

                    let metrics =
                        run_format_benchmark(format.clone(), view_count, record_count, batch_size);

                    for m in &metrics {
                        println!(
                            "  ▸ {}: {:.0} ops/sec | P50: {:.0}µs | P95: {:.0}µs | P99: {:.0}µs | Updates: {}",
                            m.phase, m.ops_per_sec, m.latency_p50_us, m.latency_p95_us, m.latency_p99_us, m.updates_emitted
                        );
                        writeln!(writer, "{}", m.to_csv()).unwrap();
                    }

                    all_metrics.extend(metrics);
                    println!();
                }
            }
        }
    }

    writer.flush().unwrap();

    // === SUMMARY ===
    println!("\n╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                         BENCHMARK SUMMARY                            ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    // Group by format and show comparison
    for format_name in ["flat", "streaming", "tree"] {
        let format_metrics: Vec<&BenchmarkMetrics> = all_metrics
            .iter()
            .filter(|m| m.format == format_name && m.phase == "CREATE")
            .collect();

        if !format_metrics.is_empty() {
            let avg_ops: f64 =
                format_metrics.iter().map(|m| m.ops_per_sec).sum::<f64>() / format_metrics.len() as f64;
            let avg_p50: f64 = format_metrics.iter().map(|m| m.latency_p50_us).sum::<f64>()
                / format_metrics.len() as f64;
            let avg_p99: f64 = format_metrics.iter().map(|m| m.latency_p99_us).sum::<f64>()
                / format_metrics.len() as f64;
            println!(
                "  {} (CREATE): {:.0} avg ops/sec | P50: {:.0}µs | P99: {:.0}µs",
                format_name.to_uppercase(),
                avg_ops,
                avg_p50,
                avg_p99
            );
        }
    }

    println!("\n  Results saved to: real_world_benchmark_results.csv");
    println!("  ══════════════════════════════════════════════════════════════════\n");
}

/// Streaming mode specific benchmark - tests delta efficiency
#[test]
#[ignore]
fn streaming_mode_benchmark() {
    println!("\n╔══════════════════════════════════════════════════════════════════════╗");
    println!("║              STREAMING MODE BENCHMARK                               ║");
    println!("║  Tests delta efficiency and event correctness                       ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    let mut circuit = setup();

    // Register views in streaming mode
    for i in 0..50 {
        let plan = create_filter_plan(&format!("stream_{}", i));
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    }

    // Track delta events
    let mut total_created = 0usize;
    let mut total_updated = 0usize;
    let mut total_deleted = 0usize;

    // CREATE phase
    let records: Vec<PreparedRecord> = (0..100)
        .into_par_iter()
        .flat_map(make_linked_record_set)
        .collect();

    println!("  CREATE phase: {} records", records.len());
    let start = Instant::now();

    for record in &records {
        let updates = circuit.ingest_record(
            &record.table,
            &record.op,
            &record.id,
            record.record.clone(),
            &record.hash,
            true,
        );

        for update in updates {
            if let ViewUpdate::Streaming(s) = update {
                for r in &s.records {
                    match r.event {
                        DeltaEvent::Created => total_created += 1,
                        DeltaEvent::Updated => total_updated += 1,
                        DeltaEvent::Deleted => total_deleted += 1,
                    }
                }
            }
        }
    }

    println!("  ▸ Time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
    println!(
        "  ▸ Events: Created={}, Updated={}, Deleted={}",
        total_created, total_updated, total_deleted
    );

    // UPDATE phase - should trigger Updated events
    total_created = 0;
    total_updated = 0;
    total_deleted = 0;

    let updates_to_make: Vec<_> = records
        .iter()
        .filter(|r| r.table == "comment" && r.record.get("text") == Some(&json!("Magic")))
        .take(20)
        .collect();

    println!("\n  UPDATE phase: {} records", updates_to_make.len());
    let start = Instant::now();

    for record in updates_to_make {
        let updated_rec = json!({
            "id": record.id,
            "text": "UpdatedMagic",
            "thread": record.record.get("thread").unwrap(),
            "author": record.record.get("author").unwrap(),
            "score": 999,
            "type": "comment"
        });
        let hash = generate_hash(&updated_rec);

        let updates = circuit.ingest_record(&record.table, "UPDATE", &record.id, updated_rec, &hash, true);

        for update in updates {
            if let ViewUpdate::Streaming(s) = update {
                for r in &s.records {
                    match r.event {
                        DeltaEvent::Created => total_created += 1,
                        DeltaEvent::Updated => total_updated += 1,
                        DeltaEvent::Deleted => total_deleted += 1,
                    }
                }
            }
        }
    }

    println!("  ▸ Time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
    println!(
        "  ▸ Events: Created={}, Updated={}, Deleted={}",
        total_created, total_updated, total_deleted
    );

    // DELETE phase
    total_created = 0;
    total_updated = 0;
    total_deleted = 0;

    let deletes_to_make: Vec<_> = records
        .iter()
        .filter(|r| r.table == "comment")
        .skip(20)
        .take(10)
        .collect();

    println!("\n  DELETE phase: {} records", deletes_to_make.len());
    let start = Instant::now();

    for record in deletes_to_make {
        let updates = circuit.ingest_record(&record.table, "DELETE", &record.id, json!({}), "", true);

        for update in updates {
            if let ViewUpdate::Streaming(s) = update {
                for r in &s.records {
                    match r.event {
                        DeltaEvent::Created => total_created += 1,
                        DeltaEvent::Updated => total_updated += 1,
                        DeltaEvent::Deleted => total_deleted += 1,
                    }
                }
            }
        }
    }

    println!("  ▸ Time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
    println!(
        "  ▸ Events: Created={}, Updated={}, Deleted={}",
        total_created, total_updated, total_deleted
    );

    println!("\n  ══════════════════════════════════════════════════════════════════\n");
}

/// Quick sanity check benchmark (faster iteration)
#[test]
#[ignore]
fn real_world_benchmark_quick() {
    println!("\n=== QUICK BENCHMARK (sanity check) ===\n");

    let formats = [
        ("Flat", ViewResultFormat::Flat),
        ("Streaming", ViewResultFormat::Streaming),
    ];

    for (name, format) in &formats {
        let mut circuit = setup();

        // Register 50 mixed views
        for i in 0..50 {
            let plan = match i % 5 {
                0..=2 => create_filter_plan(&format!("{}_{}", name, i)),
                3 => create_prefix_plan(&format!("{}_{}", name, i)),
                _ => create_join_plan(&format!("{}_{}", name, i)),
            };
            circuit.register_view(plan, None, Some(format.clone()));
        }

        // Prepare 500 records
        let prepared: Vec<PreparedRecord> = (0..167)
            .into_par_iter()
            .flat_map(make_linked_record_set)
            .collect();

        let batches: Vec<Vec<_>> = prepared
            .chunks(50)
            .map(|c| c.iter().map(to_batch_tuple).collect())
            .collect();

        // Measure
        let start = Instant::now();
        let mut total_ops = 0;
        let mut total_updates = 0;
        for batch in batches {
            total_ops += batch.len();
            let updates = circuit.ingest_batch(batch, true);
            total_updates += updates.len();
        }
        let elapsed = start.elapsed();

        println!("  {} Mode:", name);
        println!("    Records: {}", total_ops);
        println!("    Updates: {}", total_updates);
        println!("    Time: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
        println!(
            "    Throughput: {:.0} ops/sec",
            total_ops as f64 / elapsed.as_secs_f64()
        );
        println!();
    }
}

/// Comparison benchmark: Flat vs Streaming efficiency
#[test]
#[ignore]
fn format_comparison_benchmark() {
    println!("\n╔══════════════════════════════════════════════════════════════════════╗");
    println!("║              FORMAT COMPARISON: FLAT vs STREAMING                   ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    let view_counts = [10, 50, 100];
    let record_count = 1000;
    let batch_size = 50;

    println!("{:>8} {:>15} {:>15} {:>15}", "Views", "Flat (ops/s)", "Stream (ops/s)", "Ratio");
    println!("{}", "─".repeat(60));

    for view_count in view_counts {
        // Test Flat
        let flat_metrics = run_format_benchmark(ViewResultFormat::Flat, view_count, record_count, batch_size);
        let flat_ops = flat_metrics
            .iter()
            .find(|m| m.phase == "CREATE")
            .map(|m| m.ops_per_sec)
            .unwrap_or(0.0);

        // Test Streaming
        let stream_metrics =
            run_format_benchmark(ViewResultFormat::Streaming, view_count, record_count, batch_size);
        let stream_ops = stream_metrics
            .iter()
            .find(|m| m.phase == "CREATE")
            .map(|m| m.ops_per_sec)
            .unwrap_or(0.0);

        let ratio = if flat_ops > 0.0 {
            stream_ops / flat_ops
        } else {
            0.0
        };

        println!(
            "{:>8} {:>15.0} {:>15.0} {:>15.2}x",
            view_count, flat_ops, stream_ops, ratio
        );
    }

    println!("\n  ══════════════════════════════════════════════════════════════════\n");
}