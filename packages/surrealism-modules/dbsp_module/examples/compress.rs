use std::fs::File;
use std::io::{Read, Write};

fn main() -> std::io::Result<()> {
    let input_path = "target/wasm32-wasip1/debug/dbsp_module.wasm";
    let output_path = "../../../tests/.spooky/dbsp_module.surli"; // Write directly to tests folder

    let mut input_file = File::open(input_path)?;
    let mut buffer = Vec::new();
    input_file.read_to_end(&mut buffer)?;

    let compressed = zstd::encode_all(&buffer[..], 0)?; // Level 0 (default)

    let mut output_file = File::create(output_path)?;
    output_file.write_all(&compressed)?;

    println!("Compressed {} -> {}", input_path, output_path);
    Ok(())
}
