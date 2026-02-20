import re
import glob

# For tests files
for path in glob.glob("tests/**/*.rs", recursive=True):
    with open(path, 'r') as f:
        text = f.read()
        
    original = text
    
    text = text.replace("ssp::engine::types::FastMap::default()", "std::collections::BTreeMap::new()")
    text = text.replace("FastMap::default()", "std::collections::BTreeMap::new()")
    text = text.replace("SpookyValue::Number(created_at as f64)", "SpookyValue::from(created_at as f64)")
    text = text.replace("SpookyValue::Number(i as f64)", "SpookyValue::from(i as f64)")
    
    # db.clone()
    text = text.replace("db.clone();", "spooky_db_module::db::SpookyDb::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap();")
    
    # circuit.clone() in benchmark.rs
    text = text.replace("let mut thread_circuit = circuit.clone();", "let mut thread_circuit = Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap(); thread_circuit.init_load(records.clone());")
    
    # Leftover db.tables.get
    text = re.sub(r'db2\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'db2.table_len("\1")', text)
    text = re.sub(r'db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'db.table_len("\1")', text)
    text = re.sub(r'db2\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.zset\.contains_key\("([^"]+)"\)', r'(db2.get_zset_weight("\1", "\2") > 0)', text)
    text = re.sub(r'db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.zset\.contains_key\("([^"]+)"\)', r'(db.get_zset_weight("\1", "\2") > 0)', text)
    
    # Update process_delta for db2 vs circuit
    # `let result = computed.process_delta(&delta, &db2);` -> `computed.process_delta` expects `&SpookyDb`. Wait! `Circuit` was passed in `edge_test.rs`?
    # error[E0308]: mismatched types: expected `&SpookyDb`, found `&Circuit`
    text = text.replace("computed.process_delta(&delta, &db2)", "computed.process_delta(&delta, db2.get_db())")
    
    if text != original:
        with open(path, 'w') as f:
            f.write(text)

