use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn compile_modules(modules_dir: &Path, output_dir: &Path) -> Result<()> {
    if !modules_dir.exists() {
        println!("Modules directory not found: {:?}", modules_dir);
        return Ok(());
    }

    println!("Scanning for modules in {:?}", modules_dir);

    for entry in fs::read_dir(modules_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Check if it's a rust project (has Cargo.toml)
            let cargo_toml = path.join("Cargo.toml");
            if cargo_toml.exists() {
                let module_name = path.file_name().unwrap().to_string_lossy().to_string();
                println!("  Compiling module: {}", module_name);

                // Run cargo build with wasi target
                let status = Command::new("cargo")
                    .args(&["build", "--release", "--target", "wasm32-wasip1"])
                    .current_dir(&path)
                    .status()
                    .context(format!("Failed to run cargo build for module {}", module_name))?;

                if !status.success() {
                    anyhow::bail!("Compilation failed for module {}", module_name);
                }

                // Locate the WASM file
                let target_dir = path.join("target/wasm32-wasip1/release");
                let wasm_files: Vec<PathBuf> = fs::read_dir(&target_dir)
                    .context("Failed to read target directory")?
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().map_or(false, |ext| ext == "wasm"))
                    .collect();

                if wasm_files.is_empty() {
                    println!("    Warning: No WASM file found in {:?}", target_dir);
                    continue;
                }

                // Should be only one wasm file usually
                for wasm_path in wasm_files {
                    let file_name = wasm_path.file_name().unwrap().to_string_lossy();
                    // Output file name: module_name.surli
                    // OR should we respect the filename? Let's use the package name/module name.
                    // The user's output shows `xor_module.surli`.
                    let output_filename = format!("{}.surli", module_name);
                    let dest_path = output_dir.join(&output_filename);
                    
                    // Package into .surli (ZSTD compressed TAR)
                    println!("    Packaging {}...", output_filename);
                    
                    // Create TAR in memory
                    let mut tar_builder = tar::Builder::new(Vec::new());
                    
                    // Add mod.wasm
                    let mut wasm_file = fs::File::open(&wasm_path)?;
                    tar_builder.append_file("mod.wasm", &mut wasm_file)?;
                    
                    // Add surrealism.toml if exists
                    let manifest_path = path.join("surrealism.toml");
                    if manifest_path.exists() {
                        let mut manifest_file = fs::File::open(&manifest_path)?;
                        tar_builder.append_file("surrealism.toml", &mut manifest_file)?;
                    } else {
                        println!("    Warning: No surrealism.toml found for {}", module_name);
                    }
                    
                    let tar_data = tar_builder.into_inner()?;
                    
                    // Compress with ZSTD
                    let compressed_data = zstd::stream::encode_all(std::io::Cursor::new(tar_data), 0)?;
                    
                    // Write to output
                    fs::write(&dest_path, compressed_data)?;
                    
                    println!("    ✓ Packaged to {:?}", dest_path);
                }
            }
        }
    }

    Ok(())
}
