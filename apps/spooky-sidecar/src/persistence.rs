use anyhow::Context;
use spooky_stream_processor::StandardCircuit;
use std::fs;
use std::path::Path;
use tracing::{error, info};

pub fn load_circuit(path: &Path) -> StandardCircuit {
    if path.exists() {
        info!("Loading persistence file from {:?}", path);
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<StandardCircuit>(&content) {
                Ok(circuit) => {
                    info!("Successfully loaded circuit state");
                    return circuit;
                }
                Err(e) => error!("Failed to deserialize persistence file: {}", e),
            },
            Err(e) => error!("Failed to read persistence file: {}", e),
        }
    } else {
        info!("No persistence file found at {:?}, starting fresh", path);
    }
    StandardCircuit::new()
}

pub fn save_circuit(path: &Path, circuit: &StandardCircuit) -> anyhow::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create persistence directory")?;
    }

    let content = serde_json::to_string(circuit).context("Failed to serialize circuit")?;
    fs::write(path, content).context("Failed to write persistence file")?;
    info!("Saved circuit state to {:?}", path);
    Ok(())
}
