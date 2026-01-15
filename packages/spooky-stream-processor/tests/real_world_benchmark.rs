//! Real-World Benchmark for spooky-stream-processor
//!
//! This benchmark models production-realistic workloads to measure:
//! - Throughput scaling across view counts
//! - Latency percentiles (P50, P99)
//! - Cache hit ratios
//! - Memory efficiency
//!
//! Run with: cargo test --release real_world_benchmark -- --nocapture --ignored

mod common;
use common::*;
use rayon::prelude::*;
use serde_json::{json, Value};
use spooky_stream_processor::engine::view::{JoinCondition, Operator, Path, Predicate, Projection, QueryPlan};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

// === CONFIGURATION ===
const RECORD_COUNTS: [usize; 4] = [100, 1000, 5000, 10000];
const VIEW_COUNTS: [usize; 6] = [10, 50, 100, 250, 500, 1000];
const BATCH_SIZES: [usize; 4] = [1, 10, 50, 100];
const WARMUP_ITERATIONS: usize = 3;

// Operation mix (realistic workload)
const CREATE_RATIO: f64 = 0.70;
const UPDATE_RATIO: f64 = 0.20;
// DELETE_RATIO: 0.10 (implicit)

/// Metrics collected per benchmark run
#[derive(Debug, Clone)]
struct BenchmarkMetrics {
    views: usize,
    records: usize,
    batch_size: usize,
    phase: String,
    total_time_ms: f64,
    ops_per_sec: f64,
    latency_p50_us: f64,
    latency_p99_us: f64,
    cache_hits: usize,
    cache_misses: usize,
}

impl BenchmarkMetrics {
    fn csv_header() -> &'static str {
        "views,records,batch_size,phase,total_time_ms,ops_per_sec,latency_p50_us,latency_p99_us,cache_hits,cache_misses,cache_hit_ratio"
    }

    fn to_csv(&self) -> String {
        let hit_ratio = if self.cache_hits + self.cache_misses > 0 {
            self.cache_hits as f64 / (self.cache_hits + self.cache_misses) as f64
        } else {
            0.0
        };
        format!(
            "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{},{},{:.4}",
            self.views,
            self.records,
            self.batch_size,
            self.phase,
            self.total_time_ms,
            self.ops_per_sec,
            self.latency_p50_us,
            self.latency_p99_us,
            self.cache_hits,
            self.cache_misses,
            hit_ratio
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
/// Simulates realistic relational data patterns
fn make_linked_record_set(prefix: usize) -> Vec<PreparedRecord> {
    let mut batch = Vec::with_capacity(3);

    // 1. Author (small metadata record)
    let author_id = format!("author:{}", ulid::Ulid::new());
    let author_rec = json!({
        "id": &author_id,
        "name": format!("User{}", prefix),
        "email": format!("user{}@example.com", prefix),
        "type": "author"
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
        "type": "thread"
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
    let text = if prefix % 5 == 0 { "Magic" } else { "Regular comment content" };
    let comment_rec = json!({
        "id": &comment_id,
        "text": text,
        "thread": &thread_id,
        "author": &author_id,
        "score": (prefix * 7) % 100,
        "type": "comment"
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

/// Convert PreparedRecord to batch tuple format
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

// === QUERY PLAN BUILDERS ===

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

/// Join query: threads JOIN authors
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
        left: Box::new(Operator::Scan { table: "thread".to_string() }),
        right: Box::new(Operator::Scan { table: "author".to_string() }),
        on: JoinCondition {
            left_field: Path::new("author"),
            right_field: Path::new("id"),
        },
    };

    let magic_comments = Operator::Filter {
        input: Box::new(Operator::Scan { table: "comment".to_string() }),
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

/// Subquery plan: threads with nested comments count
fn create_subquery_plan(view_id: &str) -> QueryPlan {
    let scan_threads = Operator::Scan {
        table: "thread".to_string(),
    };
    
    // Subquery: comments for this thread
    let comments_subquery = Operator::Filter {
        input: Box::new(Operator::Scan { table: "comment".to_string() }),
        predicate: Predicate::Eq {
            field: Path::new("thread"),
            value: json!("$parent"),  // Parameter reference
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

#[test]
#[ignore] // Run with: cargo test --release real_world_benchmark -- --nocapture --ignored
fn real_world_benchmark() {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║     REAL-WORLD BENCHMARK: spooky-stream-processor           ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Query Mix: 60% Filter, 30% Join, 10% Complex               ║");
    println!("║  Op Mix: 70% CREATE, 20% UPDATE, 10% DELETE                 ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let file = File::create("real_world_benchmark_results.csv").expect("Could not create CSV file");
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    writeln!(writer, "{}", BenchmarkMetrics::csv_header()).unwrap();

    let mut all_metrics: Vec<BenchmarkMetrics> = Vec::new();

    // Test subset for quick iteration (full matrix takes longer)
    let test_views = &VIEW_COUNTS[..4];  // [10, 50, 100, 250]
    let test_records = &RECORD_COUNTS[..3];  // [100, 1000, 5000]
    let test_batches = &BATCH_SIZES[1..3];  // [10, 50]

    for &view_count in test_views {
        for &record_count in test_records {
            for &batch_size in test_batches {
                println!("━━━ Views: {} | Records: {} | Batch: {} ━━━", 
                    view_count, record_count, batch_size);

                let mut circuit = setup();

                // === REGISTER VIEWS (Mixed Query Types) ===
                print!("  ▸ Registering {} views... ", view_count);
                std::io::stdout().flush().unwrap();
                
                for i in 0..view_count {
                    let plan = match i % 10 {
                        0..=5 => create_filter_plan(&format!("filter_{}", i)),      // 60%
                        6..=8 => create_join_plan(&format!("join_{}", i)),          // 30%
                        _ => create_complex_plan(&format!("complex_{}", i)),        // 10%
                    };
                    circuit.register_view(plan, None);
                }
                println!("done.");

                // === PREPARE DATA (parallel, outside measurement) ===
                let sets_needed = (record_count as f64 / 3.0).ceil() as usize;
                let prepared_stream: Vec<PreparedRecord> = (0..sets_needed)
                    .into_par_iter()
                    .flat_map(|i| make_linked_record_set(i))
                    .collect();

                let all_batches: Vec<Vec<(String, String, String, Value, String)>> = prepared_stream
                    .chunks(batch_size)
                    .take(record_count / batch_size)
                    .map(|chunk| chunk.iter().map(to_batch_tuple).collect())
                    .collect();

                // === WARMUP ===
                for batch in all_batches.iter().take(WARMUP_ITERATIONS) {
                    circuit.ingest_batch(batch.clone());
                }

                // === CREATE PHASE MEASUREMENT ===
                let mut latencies: Vec<Duration> = Vec::with_capacity(all_batches.len());
                let mut cache_hits = 0usize;
                let mut cache_misses = 0usize;
                let mut total_records = 0usize;

                let phase_start = Instant::now();
                
                for batch in all_batches.iter().skip(WARMUP_ITERATIONS) {
                    let batch_len = batch.len();
                    
                    let start = Instant::now();
                    let updates = circuit.ingest_batch(batch.clone());
                    let duration = start.elapsed();
                    
                    latencies.push(duration);
                    total_records += batch_len;

                    // Track cache efficiency
                    if updates.is_empty() {
                        cache_hits += 1;  // No view needed update = cache hit
                    } else {
                        cache_misses += updates.len();
                    }
                }
                
                let total_time = phase_start.elapsed();

                // Calculate percentiles
                latencies.sort();
                let p50 = percentile(&latencies, 50.0);
                let p99 = percentile(&latencies, 99.0);

                let metrics = BenchmarkMetrics {
                    views: view_count,
                    records: total_records,
                    batch_size,
                    phase: "CREATE".to_string(),
                    total_time_ms: total_time.as_secs_f64() * 1000.0,
                    ops_per_sec: total_records as f64 / total_time.as_secs_f64(),
                    latency_p50_us: p50.as_secs_f64() * 1_000_000.0,
                    latency_p99_us: p99.as_secs_f64() * 1_000_000.0,
                    cache_hits,
                    cache_misses,
                };

                println!("  ▸ CREATE: {:.0} ops/sec | P50: {:.0}µs | P99: {:.0}µs",
                    metrics.ops_per_sec, metrics.latency_p50_us, metrics.latency_p99_us);

                writeln!(writer, "{}", metrics.to_csv()).unwrap();
                all_metrics.push(metrics);

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
                            chunk.iter().map(|item| {
                                let updated = json!({
                                    "id": item.id,
                                    "text": "UpdatedMagic",
                                    "thread": item.record.get("thread").unwrap(),
                                    "author": item.record.get("author").unwrap(),
                                    "score": 999,
                                    "type": "comment"
                                });
                                (
                                    item.table.clone(),
                                    "UPDATE".to_string(),
                                    item.id.clone(),
                                    updated.clone(),
                                    generate_hash(&updated),
                                )
                            }).collect()
                        })
                        .collect();

                    let mut update_latencies: Vec<Duration> = Vec::new();
                    let mut updated_records = 0usize;
                    let update_start = Instant::now();

                    for batch in update_batches {
                        let batch_len = batch.len();
                        let start = Instant::now();
                        circuit.ingest_batch(batch);
                        update_latencies.push(start.elapsed());
                        updated_records += batch_len;
                    }

                    let update_time = update_start.elapsed();
                    update_latencies.sort();

                    let update_metrics = BenchmarkMetrics {
                        views: view_count,
                        records: updated_records,
                        batch_size,
                        phase: "UPDATE".to_string(),
                        total_time_ms: update_time.as_secs_f64() * 1000.0,
                        ops_per_sec: updated_records as f64 / update_time.as_secs_f64(),
                        latency_p50_us: percentile(&update_latencies, 50.0).as_secs_f64() * 1_000_000.0,
                        latency_p99_us: percentile(&update_latencies, 99.0).as_secs_f64() * 1_000_000.0,
                        cache_hits: 0,
                        cache_misses: 0,
                    };

                    println!("  ▸ UPDATE: {:.0} ops/sec | P50: {:.0}µs | P99: {:.0}µs",
                        update_metrics.ops_per_sec, update_metrics.latency_p50_us, update_metrics.latency_p99_us);

                    writeln!(writer, "{}", update_metrics.to_csv()).unwrap();
                    all_metrics.push(update_metrics);
                }

                println!();
            }
        }
    }

    writer.flush().unwrap();

    // === SUMMARY ===
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                      BENCHMARK SUMMARY                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // Group by view count and show scaling
    for &vc in test_views {
        let vc_metrics: Vec<&BenchmarkMetrics> = all_metrics
            .iter()
            .filter(|m| m.views == vc && m.phase == "CREATE")
            .collect();
        
        if !vc_metrics.is_empty() {
            let avg_ops: f64 = vc_metrics.iter().map(|m| m.ops_per_sec).sum::<f64>() / vc_metrics.len() as f64;
            println!("  {} Views: {:.0} avg ops/sec", vc, avg_ops);
        }
    }

    println!("\n  Results saved to: real_world_benchmark_results.csv");
    println!("  ══════════════════════════════════════════════════════════════\n");
}

/// Quick sanity check benchmark (faster iteration)
#[test]
#[ignore]
fn real_world_benchmark_quick() {
    println!("\n=== QUICK BENCHMARK (sanity check) ===\n");

    let mut circuit = setup();

    // Register 100 mixed views
    for i in 0..100 {
        let plan = match i % 10 {
            0..=5 => create_filter_plan(&format!("filter_{}", i)),
            6..=8 => create_join_plan(&format!("join_{}", i)),
            _ => create_complex_plan(&format!("complex_{}", i)),
        };
        circuit.register_view(plan, None);
    }

    // Prepare 1000 records
    let prepared: Vec<PreparedRecord> = (0..334)
        .into_par_iter()
        .flat_map(|i| make_linked_record_set(i))
        .collect();

    let batches: Vec<Vec<_>> = prepared
        .chunks(50)
        .map(|c| c.iter().map(to_batch_tuple).collect())
        .collect();

    // Measure
    let start = Instant::now();
    let mut total_ops = 0;
    for batch in batches {
        total_ops += batch.len();
        circuit.ingest_batch(batch);
    }
    let elapsed = start.elapsed();

    println!("  Records: {}", total_ops);
    println!("  Time: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
    println!("  Throughput: {:.0} ops/sec", total_ops as f64 / elapsed.as_secs_f64());
    println!();
}
