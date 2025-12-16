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

                // Run cargo build
                let status = Command::new("cargo")
                    .args(&["build", "--release", "--target", "wasm32-unknown-unknown"])
                    .current_dir(&path)
                    .status()
                    .context(format!("Failed to run cargo build for module {}", module_name))?;

                if !status.success() {
                    anyhow::bail!("Compilation failed for module {}", module_name);
                }

                // Locate the WASM file
                // Usually in target/wasm32-unknown-unknown/release/*.wasm
                // The filename normally matches the package name in Cargo.toml.
                // However, user example shows "spooky-xor_module-0.1.0.surli" which implies some renaming or specific naming.
                // For now, I'll look for any .wasm file in the release dir.
                
                let target_dir = path.join("target/wasm32-unknown-unknown/release");
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

                // Copy to output dir with .surli extension
                // If multiple, copy all (though usually one per project)
                for wasm_path in wasm_files {
                    let file_name = wasm_path.file_name().unwrap().to_string_lossy();
                    // Rename .wasm to .surli
                    let new_file_name = format!("{}.surli", file_name.trim_end_matches(".wasm"));
                    let dest_path = output_dir.join(&new_file_name);
                    
                    fs::copy(&wasm_path, &dest_path)
                        .context(format!("Failed to copy WASM file to {:?}", dest_path))?;
                    
                    println!("    ✓ Copied {} to {:?}", file_name, dest_path);
                }
            }
        }
    }

    Ok(())
}
