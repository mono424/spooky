import re
import glob

for path in glob.glob("tests/**/*.rs", recursive=True):
    with open(path, 'r') as f:
        text = f.read()
        
    text = text.replace('Some(&bytes)', 'Some(&bytes.0)')
    
    with open(path, 'w') as f:
        f.write(text)

with open("benches/memory_bench.rs", "r") as f:
    text = f.read()

# Instead of `base_circuit` from the outside which cannot be cloned:
# let's just create a new circuit inside with_inputs
# Actually, the base_circuit is already pre-loaded with initial records!
# If we can't clone Circuit, we have to reload the records inside `with_inputs`.
# But `with_inputs` is run outside the timed loop. So that's perfectly fine!

# Let's replace `base_circuit` with a call to `setup_base_circuit()` 
# First we need to extract `setup_base_circuit` logic.
text = text.replace("|| (base_circuit, new_entries.clone())", "|| (setup_base_circuit(NUM_RECORDS), new_entries.clone())")
text = text.replace("|| (base_circuit, plan.clone())", "|| (setup_base_circuit(NUM_RECORDS), plan.clone())")
text = text.replace("|| (base_circuit, comment_entries.clone())", "|| (setup_base_circuit(NUM_RECORDS), comment_entries.clone())")

# Now inject `setup_base_circuit`
setup_func = """
fn setup_base_circuit(num_records: usize) -> ssp::engine::circuit::Circuit {
    let mut circuit = ssp::engine::circuit::Circuit::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap();
    let records = (0..num_records)
        .map(|i| {
            let mut data = ssp::engine::types::FastMap::default();
            data.insert(smol_str::SmolStr::new("id"), ssp::engine::types::SpookyValue::Str(smol_str::SmolStr::new(format!("user:{}", i))));
            data.insert(smol_str::SmolStr::new("type"), ssp::engine::types::SpookyValue::Str(smol_str::SmolStr::new("user")));
            data.insert(smol_str::SmolStr::new("score"), ssp::engine::types::SpookyValue::Number(i as f64));
            
            ssp::engine::types::LoadRecord {
                table: "perf".to_string(),
                id: format!("user:{}", i),
                data: ssp::engine::types::SpookyValue::Object(data),
                spooky_rv: None,
            }
        })
        .collect::<Vec<_>>();
    circuit.init_load(records);
    circuit
}
"""
text = text.replace("#[divan::bench", setup_func + "\n#[divan::bench", 1)

with open("benches/memory_bench.rs", "w") as f:
    f.write(text)

