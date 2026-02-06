use divan::{black_box, AllocProfiler, Bencher};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use ssp::engine::circuit::dto::BatchEntry;
use ssp::engine::circuit::Circuit;
use ssp::{Operator, QueryPlan, SpookyValue};
use std::fs::File;
use std::io::{BufRead, BufReader};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

// --------------------------------------------------------------------------
// 1. Die Structs (Müssen exakt zum Generator passen, aber mit Deserialize)
// --------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct User {
    id: String,
    age: usize,
    admin: bool,
    username: String, // Hier steckt das Padding drin
}

#[derive(Serialize, Deserialize)]
struct Thread {
    id: String,
    title: String,
    author: String,
    active: bool,
    created_at: String,
    content: String, // Hier steckt das Padding drin
}

#[derive(Serialize, Deserialize)]
struct Comment {
    id: String,
    thread: String,
    author: String,
    created_at: String,
    content: String, // Hier steckt das Padding drin
}

// --------------------------------------------------------------------------
// 2. Die Ingest-Funktionen (Generic)
// --------------------------------------------------------------------------

fn read_users(limit: Option<u32>) -> Vec<User> {
    // Öffne Datei (Panic wenn nicht da - bitte erst Generator laufen lassen!)
    let file = File::open("users.jsonl").expect("users.jsonl nicht gefunden");
    let reader = BufReader::new(file);

    let mut users: Vec<User> = vec![];
    let mut count: u32 = 0;
    for line in reader.lines() {
        if limit.is_some() && Some(count) >= limit {
            break;
        };
        count = count + 1;
        let line_str = line.unwrap();
        // Hier passiert die Magie: String -> Struct
        let user: User = serde_json::from_str(&line_str).unwrap();
        users.push(user);
    }
    users
}

fn read_threads(limit: Option<u32>) -> Vec<Thread> {
    let file = File::open("threads.jsonl").expect("threads.jsonl nicht gefunden");
    let reader = BufReader::new(file);

    let mut threads: Vec<Thread> = vec![];
    let mut count: u32 = 0;
    for line in reader.lines() {
        if limit.is_some() && Some(count) >= limit {
            break;
        };
        count = count + 1;
        let line_str = line.unwrap();
        let thread: Thread = serde_json::from_str(&line_str).unwrap();
        threads.push(thread);
    }
    threads
}

fn read_comments(limit: Option<u32>) -> Vec<Comment> {
    let file = File::open("comments.jsonl").expect("comments.jsonl nicht gefunden");
    let reader = BufReader::new(file);

    let mut comments: Vec<Comment> = vec![];
    let mut count: u32 = 0;
    for line in reader.lines() {
        if limit.is_some() && Some(count) >= limit {
            break;
        };
        count = count + 1;
        let line_str = line.unwrap();
        let comment: Comment = serde_json::from_str(&line_str).unwrap();
        comments.push(comment);
    }
    comments
}

fn setup(limit: Option<u32>) -> Vec<BatchEntry> {
    let records = read_comments(limit);
    let prepared_entries: Vec<BatchEntry> = records
        .iter()
        .map(|record| {
            let id = record.id.clone();
            let json_string = serde_json::to_string(record).unwrap();
            let data = SpookyValue::Str(SmolStr::new(json_string));

            BatchEntry::create("comment", id, data)
        })
        .collect();
    prepared_entries
}

// --------------------------------------------------------------------------
// 3. Der Benchmark
// --------------------------------------------------------------------------

#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_ingest_0(bencher: Bencher) {
    let records = read_users(Some(100));
    let new_entries: Vec<BatchEntry> = records
        .iter()
        .map(|record| {
            let id = record.id.clone();
            let json_string = serde_json::to_string(record).unwrap();
            let data = SpookyValue::Str(SmolStr::new(json_string));

            BatchEntry::new("user", ssp::engine::types::Operation::Create, id, data)
        })
        .collect();
    bencher
        .with_inputs(|| (Circuit::new(), new_entries.clone()))
        .bench_values(|(mut circuit, entries)| {
            for entry in entries {
                circuit.ingest_single(entry);
            }
            // Verhindert Weg-Optimierung
            black_box(circuit);
        });
}

#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_ingest_10k(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    // A) Den "schweren" Basis-Circuit mit 10.000 Items bauen
    let initial_data = setup(Some(10_000));
    let mut base_circuit = Circuit::new();
    for entry in initial_data {
        base_circuit.ingest_single(entry);
    }

    // B) Die "neuen" 100 Items vorbereiten
    let records = read_users(Some(100));
    let new_entries: Vec<BatchEntry> = records
        .iter()
        .map(|record| {
            let id = record.id.clone();
            let json_string = serde_json::to_string(record).unwrap();
            let data = SpookyValue::Str(SmolStr::new(json_string));
            BatchEntry::create("user", id, data)
        })
        .collect();

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), new_entries.clone()))
        .bench_values(|(mut circuit, entries)| {
            for entry in entries {
                circuit.ingest_single(entry);
            }
            // Verhindert Weg-Optimierung
            black_box(circuit);
        });
}

#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_ingest_100k(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    // A) Den "schweren" Basis-Circuit mit 10.000 Items bauen
    let initial_data = setup(Some(100_000));
    let mut base_circuit = Circuit::new();
    for entry in initial_data {
        base_circuit.ingest_single(entry);
    }

    // B) Die "neuen" 100 Items vorbereiten
    let records = read_users(Some(100));
    let new_entries: Vec<BatchEntry> = records
        .iter()
        .map(|record| {
            let id = record.id.clone();
            let json_string = serde_json::to_string(record).unwrap();
            let data = SpookyValue::Str(SmolStr::new(json_string));
            BatchEntry::create("user", id, data)
        })
        .collect();

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), new_entries.clone()))
        .bench_values(|(mut circuit, entries)| {
            for entry in entries {
                circuit.ingest_single(entry);
            }
            // Verhindert Weg-Optimierung
            black_box(circuit);
        });
}
#[allow(dead_code)]
//#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_ingest_1000k(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    // A) Den "schweren" Basis-Circuit mit 10.000 Items bauen
    let initial_data = setup(Some(1_000_000));
    let mut base_circuit = Circuit::new();
    for entry in initial_data {
        base_circuit.ingest_single(entry);
    }

    // B) Die "neuen" 100 Items vorbereiten
    let records = read_users(Some(100));
    let new_entries: Vec<BatchEntry> = records
        .iter()
        .map(|record| {
            let id = record.id.clone();
            let json_string = serde_json::to_string(record).unwrap();
            let data = SpookyValue::Str(SmolStr::new(json_string));
            BatchEntry::create("user", id, data)
        })
        .collect();

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), new_entries.clone()))
        .bench_values(|(mut circuit, entries)| {
            for entry in entries {
                circuit.ingest_single(entry);
            }
            // Verhindert Weg-Optimierung
            black_box(circuit);
        });
}

#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_register_0(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    let base_circuit = Circuit::new();
    let plan = QueryPlan {
        id: "view:1".to_string(),
        root: Operator::Limit {
            input: Box::new(Operator::Scan {
                table: "comment".to_string(),
            }),
            limit: 10 as usize,
            order_by: Some(vec![]),
        },
    };

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), plan.clone()))
        .bench_values(|(mut circuit, plan)| {
            circuit.register_view(plan, None, None);
            black_box(circuit);
        });
}

#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_register_1k(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    let initial_data = setup(Some(1_000));
    let mut base_circuit = Circuit::new();
    for entry in initial_data {
        base_circuit.ingest_single(entry);
    }
    let plan = QueryPlan {
        id: "view:1".to_string(),
        root: Operator::Limit {
            input: Box::new(Operator::Scan {
                table: "comment".to_string(),
            }),
            limit: 10 as usize,
            order_by: Some(vec![]),
        },
    };

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), plan.clone()))
        .bench_values(|(mut circuit, plan)| {
            circuit.register_view(plan, None, None);
            black_box(circuit);
        });
}

#[allow(dead_code)]
//#[divan::bench(sample_count = 1_000, sample_size = 1)]
fn bench_register_10k(bencher: Bencher) {
    // ======================================================
    // 1. GLOBAL SETUP (Einmalig ganz am Anfang)
    // ======================================================

    let initial_data = setup(Some(10_000));
    let mut base_circuit = Circuit::new();
    for entry in initial_data {
        base_circuit.ingest_single(entry);
    }
    let plan = QueryPlan {
        id: "view:1".to_string(),
        root: Operator::Limit {
            input: Box::new(Operator::Scan {
                table: "comment".to_string(),
            }),
            limit: 10 as usize,
            order_by: Some(vec![]),
        },
    };

    // ======================================================
    // 2. INPUT & MESSUNG
    // ======================================================
    bencher
        .with_inputs(|| (base_circuit.clone(), plan.clone()))
        .bench_values(|(mut circuit, plan)| {
            circuit.register_view(plan, None, None);
            black_box(circuit);
        });
}
