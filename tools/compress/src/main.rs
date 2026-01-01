use std::fs::File;
use std::io::{Read, Write};

fn main() -> std::io::Result<()> {
    // Relative paths from tools/compress
    let wasm_path = "../../packages/surrealism-modules/dbsp_module/target/wasm32-wasip1/debug/dbsp_module.wasm";
    let tar_path = "dbsp_module.tar";
    let output_path = "../../tests/.spooky/dbsp_module.surli"; 

    println!("Creating tarball from: {}", wasm_path);
    
    // Copy to local mod.wasm so tar contains "mod.wasm"
    let local_wasm = "mod.wasm";
    std::fs::copy(wasm_path, local_wasm)?;

    // Also copy surrealism.toml
    let toml_path = "../../packages/surrealism-modules/dbsp_module/surrealism.toml";
    std::fs::copy(toml_path, "surrealism.toml")?;

    let status = std::process::Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-c")
        .arg("-f")
        .arg(tar_path)
        .arg(local_wasm)
        .arg("surrealism.toml")
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "tar failed"));
    }
    
    // Clean up local copy
    std::fs::remove_file(local_wasm)?;
    std::fs::remove_file("surrealism.toml")?;

    println!("Reading tarball...");
    let mut input_file = File::open(tar_path)?;
    let mut buffer = Vec::new();
    input_file.read_to_end(&mut buffer)?;

    println!("Compressing {} bytes (tar)...", buffer.len());
    let compressed = zstd::encode_all(&buffer[..], 0)?; // Level 0 (default)

    println!("Writing {} bytes to: {}", compressed.len(), output_path);
    let mut output_file = File::create(output_path)?;
    output_file.write_all(&compressed)?;

    // Clean up tar
    std::fs::remove_file(tar_path)?;

    println!("Done.");
    Ok(())
}
