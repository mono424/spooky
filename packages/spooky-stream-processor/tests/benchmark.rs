mod common;
use common::*;
use rayon::prelude::*;
use serde_json::json;
use spooky_stream_processor::engine::view::{JoinCondition, Operator, Path, Predicate, QueryPlan};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::time::{Duration, Instant};

/*
#[cfg(not(target_arch = "wasm32"))]
use mimalloc::MiMalloc;

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
*/

// --- KONFIGURATION ---
const TOTAL_RECORDS: usize = 10;
const VIEW_COUNTS: [usize; 4] = [10, 100, 500, 1000];
const BATCH_SIZE: usize = 10; // Wir messen exakt jeden 50er Block

struct PreparedRecord {
    table: String,
    op: String,
    id: String,
    record: serde_json::Value,
    hash: String,
}

#[test]
fn benchmark_latency_mixed_stream() {
    let file = File::create("benchmark_results.csv").expect("Konnte CSV Datei nicht erstellen");
    // Großer Puffer für CSV Schreiben, um I/O Bremse zu vermeiden
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    // Header schreiben
    // latency_last_50_ms: Wie lange hat ein einzelner Record im letzten 50er Batch durchschnittlich gebraucht?
    // ops_per_sec: Wie viele Records schaffen wir pro Sekunde (insgesamt)?
    writeln!(
        writer,
        "views,records,total_time_ms,latency_last_50_ms,ops_per_sec"
    )
    .unwrap();

    println!(
        "Start Benchmark ({} Records Total, Batch Size {})...",
        TOTAL_RECORDS, BATCH_SIZE
    );

    for &view_count in &VIEW_COUNTS {
        let mut circuit = setup();

        // --- 1. Views Setup ---
        print!(">> Setup {} Views... ", view_count);
        io::stdout().flush().unwrap();
        for i in 0..view_count {
            let plan = create_magic_comments_plan(&format!("view_{}", i));
            circuit.register_view(plan, None);
        }
        println!("Fertig.");

        // --- 2. Daten Vorbereitung (Parallel mit Rayon) ---
        let sets_needed = (TOTAL_RECORDS as f64 / 3.0).ceil() as usize;

        let mut prepared_stream: Vec<PreparedRecord> = (0..sets_needed)
            .into_par_iter()
            .map(|_| {
                let mut batch = Vec::with_capacity(3);

                // 1. Author
                let (auth_id, rec_auth) = make_author_record("BenchUser");
                batch.push(PreparedRecord {
                    table: "author".to_string(),
                    op: "CREATE".to_string(),
                    id: auth_id.clone(),
                    hash: generate_hash(&rec_auth),
                    record: rec_auth,
                });

                // 2. Thread
                // For benchmark, we need specific linkage, so we generate IDs first?
                // Wait, make_* helpers generate random IDs internally.
                // We need to link them. The current helpers in common/mod.rs GENERATE new IDs.
                // We need to refactor common/mod.rs again to allow passing IDs or return them.
                // Actually, make_* returns (id, record). So we can use that!

                // DATA FLOW:
                // Author -> (author_id, author_rec)
                // Thread -> needs author_id. returns (thread_id, thread_rec)
                // Comment -> needs thread_id, author_id. returns (comment_id, comment_rec)

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
                let (comment_id, rec_comment) =
                    make_comment_record("Magic", &thread_id, &author_id);
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

        // Exakt auf gewünschte Länge schneiden
        prepared_stream.truncate(TOTAL_RECORDS); // Should be enough as we map sets of 3

        // Referenz für Verifizierung (letzter Thread mit Comment)
        let last_valid_thread_id = prepared_stream
            .iter()
            .rev()
            .find(|r| r.table == "comment")
            .map(|r| {
                r.record
                    .get("thread")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .expect("Keine Kommentare im Stream gefunden!");

        // --- 3. Ingest Loop & Messung ---
        let mut total_ingest_duration = Duration::new(0, 0);
        let mut global_record_count = 0;

        for chunk in prepared_stream.chunks(BATCH_SIZE) {
            // Daten für Batch vorbereiten (nicht messen)
            let batch_data: Vec<(String, String, String, serde_json::Value, String)> = chunk
                .iter()
                .map(|item| {
                    (
                        item.table.clone(),
                        item.op.clone(),
                        item.id.clone(),
                        item.record.clone(),
                        item.hash.clone(),
                    )
                })
                .collect();

            let batch_len = batch_data.len();

            // --- MESSUNG START ---
            let start = Instant::now();
            let muvs = circuit.ingest_batch(batch_data); // Ruft jetzt die optimierte Batch-Methode auf
                                                         //println!("{:#?}", muvs);
            let duration = start.elapsed();
            // --- MESSUNG ENDE ---

            total_ingest_duration += duration;
            global_record_count += batch_len;

            // --- BERECHNUNG ---
            let total_ms = total_ingest_duration.as_secs_f64() * 1000.0;

            // 1. Latenz pro Record (nur für diesen Batch)
            // Wenn der Batch 50ms gedauert hat und 50 Records hatte -> 1ms Latency
            let latency_last_50_ms = (duration.as_secs_f64() * 1000.0) / batch_len as f64;

            // 2. Durchsatz (Global)
            // Anzahl aller Records / Gesamte Zeit
            let ops_sec = global_record_count as f64 / total_ingest_duration.as_secs_f64();

            writeln!(
                writer,
                "{},{},{:.2},{:.4},{:.2}",
                view_count, global_record_count, total_ms, latency_last_50_ms, ops_sec
            )
            .unwrap();

            // Terminal Anzeige (Live Update)
            print!(
                "\r>> Views: {} | Records: {}/{} | Latency: {:.3} ms | Speed: {:.0} ops/sec",
                view_count, global_record_count, TOTAL_RECORDS, latency_last_50_ms, ops_sec
            );
            io::stdout().flush().unwrap();
        }

        // --- 4. Verifizierung ---
        let view = circuit
            .views
            .iter()
            .find(|v| v.plan.id == "view_0")
            .unwrap();
        if !view.cache.contains_key(last_valid_thread_id.as_str()) {
            panic!(
                "\nFEHLER: Circuit Update unvollständig! Thread {} fehlt im View.",
                last_valid_thread_id
            );
        }
        println!("{:#?}", circuit.clone());
    }
    println!("Benchmark abgeschlossen. Ergebnisse in 'benchmark_results.csv'.");
}

// Helper für Plan
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
