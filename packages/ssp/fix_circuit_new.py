import re
import glob

for path in glob.glob("tests/**/*.rs", recursive=True):
    with open(path, 'r') as f:
        text = f.read()

    original_text = text

    # Fix any remaining Circuit::new() without arguments
    text = text.replace("Circuit::new()", "Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")
    
    # Fix Result handling if the user assigned `let mut circuit = Circuit::new(path);` without unwrap
    # In benches/memory_bench.rs we had `no method named clone found for struct Circuit`
    # Wait, the error is `circuit.clone(); method not found in Circuit`... wait, did the user call clone on circuit?
    # Yes, tests/benchmark.rs:130: `circuit.clone()`. Circuit does not implement Clone! It holds `db: SpookyDb` which contains `RedbDatabase` which does not implement Clone if we didn't add it.
    
    if text != original_text:
        with open(path, 'w') as f:
            f.write(text)

with open("benches/memory_bench.rs", "r") as f:
    text = f.read()
text = text.replace("Circuit::new()", "Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")
text = text.replace("let circuit = Circuit::new", "let mut circuit = Circuit::new")
text = text.replace("circuit.clone()", "circuit") # if it's there
with open("benches/memory_bench.rs", "w") as f:
    f.write(text)
    
with open("tests/benchmark.rs", "r") as f:
    text = f.read()
text = text.replace("Circuit::new()", "Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")    
text = text.replace("let mut thread_circuit = circuit.clone();", "let mut thread_circuit = Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap(); thread_circuit.init_load(records.clone());")
with open("tests/benchmark.rs", "w") as f:
    f.write(text)
    
