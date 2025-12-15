use surrealism::surrealism;

/// XOR two BLAKE3 hex strings and return the result as a hex string
///
/// # Arguments
/// * `hash1` - First BLAKE3 hash as hex string
/// * `hash2` - Second BLAKE3 hash as hex string
///
/// # Returns
/// * `Ok(String)` - XOR result as hex string
/// * `Err(&'static str)` - Error message if decoding fails or lengths don't match
#[surrealism]
fn blake3_xor(hash1: String, hash2: String) -> Result<String, &'static str> {
    // Decode hex strings to bytes
    let bytes1 = hex::decode(&hash1)
        .map_err(|_| "Failed to decode first hash from hex")?;
    let bytes2 = hex::decode(&hash2)
        .map_err(|_| "Failed to decode second hash from hex")?;

    // Check if lengths match
    if bytes1.len() != bytes2.len() {
        return Err("Hash lengths do not match");
    }

    // Perform XOR operation
    let result: Vec<u8> = bytes1.iter()
        .zip(bytes2.iter())
        .map(|(a, b)| a ^ b)
        .collect();

    // Encode result back to hex
    Ok(hex::encode(result))
}
