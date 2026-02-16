use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};

// Wir nutzen ein rekursives Enum, um beliebige Strukturen zu bauen
#[derive(Serialize)]
#[serde(untagged)] // Wichtig! Damit im CBOR keine Enum-Namen stehen
enum Node {
    Simple(String),
    Nested(Box<BTreeMap<String, Node>>),
}

// Erzeugt eine tiefe Verschachtelung: { "down": { "down": { ... } } }
fn generate_deep_trap(depth: usize) -> Node {
    if depth == 0 {
        return Node::Simple("Bottom of the pit".to_string());
    }

    let mut map = BTreeMap::new();
    map.insert("down".to_string(), generate_deep_trap(depth - 1));
    Node::Nested(Box::new(map))
}

fn main() {
    let target_items = 15_000; // Anzahl der Felder im Haupt-Objekt
    let deep_frequency = 2000; // Alle 2000 Felder kommt ein tiefes Loch
    let max_depth = 200; // Wie tief das Loch ist

    // Wir nutzen BTreeMap damit die Keys sortiert sind (deterministisch für Benchmarks)
    let mut root_map = BTreeMap::new();

    println!("Generiere Map mit {} Einträgen...", target_items);

    for i in 0..target_items {
        let key = format!("key_{:05}", i); // z.B. "key_00001"

        if i > 0 && i % deep_frequency == 0 {
            // Hier fügen wir die "Deep Nested" Struktur ein
            root_map.insert(key, generate_deep_trap(max_depth));
        } else {
            // Hier ist der flache "id": "timothy" Teil
            // Wir machen den String etwas länger, um Bytes zu füllen
            root_map.insert(
                key,
                Node::Simple("Timothy_is_testing_surrealdb_performance".to_string()),
            );
        }
    }

    // Serialisieren
    let cbor_bytes = cbor4ii::serde::to_vec(Vec::new(), &root_map).unwrap();

    let size_kb = cbor_bytes.len() as f64 / 1024.0;
    println!("CBOR Größe: {} Bytes ({:.2} KB)", cbor_bytes.len(), size_kb);

    // Als .rs Datei speichern
    let file = File::create("cbor_flat_map.rs").unwrap();
    let mut writer = BufWriter::new(file);

    writeln!(
        writer,
        "/// Generated Map: {} fields, mixed depth",
        target_items
    )
    .unwrap();
    writeln!(writer, "pub const BENCH_MAP: &[u8] = &[",).unwrap();
    for (i, byte) in cbor_bytes.iter().enumerate() {
        if i % 16 == 0 {
            write!(writer, "\n    ").unwrap();
        }
        write!(writer, "0x{:02x}, ", byte).unwrap();
    }
    writeln!(writer, "\n];").unwrap();

    println!("✅ Datei 'cbor_flat_map.rs' erstellt!");
}
