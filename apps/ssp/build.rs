//! Bakes a human-readable UTC build timestamp into the binary so the
//! startup banner can show whether the running process is a fresh local
//! `cargo build` or a long-published Docker Hub image. Cross-checked at
//! runtime via `env!("SPOOKY_BUILD_TIMESTAMP")`.

fn main() {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    println!("cargo:rustc-env=SPOOKY_BUILD_TIMESTAMP={}", now);

    // Force the build script to rerun on every build so the embedded
    // timestamp tracks the actual link time rather than getting frozen
    // on the first invocation. Pointing rerun-if-changed at a path that
    // never exists is the documented cargo idiom for "always rerun".
    println!("cargo:rerun-if-changed=build-rs-always-rerun");
}
