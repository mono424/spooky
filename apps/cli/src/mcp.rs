use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::backend::{self, ResolvedSurrealDb, DEFAULT_CONFIG_PATH};

/// Resolve the path to the mcp-proxy entry point.
fn resolve_mcp_proxy() -> Result<PathBuf> {
    // 1. Sibling package in monorepo dev layout:
    //    cli/src/mcp.rs  →  cli/  →  ../mcp-proxy/dist/index.js
    let exe = std::env::current_exe().unwrap_or_default();
    // When run via `node dist/cli.js`, __dirname is apps/cli/dist
    // The Rust binary lives in apps/cli/target/release/ or next to dist/
    // Try several common layouts:

    let exe_dir = exe.parent().unwrap_or(Path::new("."));

    let candidates = [
        // npx: binary at node_modules/@spooky-sync/cli-darwin-arm64/spooky
        //       mcp-proxy at node_modules/@spooky-sync/cli/mcp-proxy/
        exe_dir.join("../../cli/mcp-proxy/index.js"),
        // Bundled: mcp-proxy/ sits next to the binary
        exe_dir.join("../mcp-proxy/index.js"),
        // Dev monorepo: relative to CWD
        PathBuf::from("apps/cli/mcp-proxy/index.js"),
        // Dev monorepo: binary at apps/cli/target/release/spooky
        exe_dir.join("../../../mcp-proxy/dist/index.js"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.canonicalize().unwrap_or_else(|_| candidate.clone()));
        }
    }

    // 2. Try node resolution as last resort
    let output = Command::new("node")
        .args(["-e", "console.log(require.resolve('@spooky-sync/mcp-proxy'))"])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    bail!(
        "Could not find @spooky-sync/mcp-proxy. Checked:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n  \
         - {}\n  \
         - node require.resolve()\n\
         Try: pnpm install && pnpm --filter @spooky-sync/mcp-proxy build",
        candidates[0].display(),
        candidates[1].display(),
        candidates[2].display(),
        candidates[3].display(),
    )
}

pub fn run() -> Result<()> {
    let mcp_proxy_path =
        resolve_mcp_proxy().context("Failed to locate MCP proxy server")?;

    // Load sp00ky.yml for SurrealDB connection defaults
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let db = ResolvedSurrealDb::from_config(&config.surrealdb);

    // In dev mode, SurrealDB runs on port 8666
    let surreal_url = std::env::var("SURREAL_URL")
        .unwrap_or_else(|_| format!("http://localhost:8666"));

    let mut child = Command::new("node")
        .arg(&mcp_proxy_path)
        .env("SURREAL_URL", &surreal_url)
        .env("SURREAL_NS", &db.namespace)
        .env("SURREAL_DB", &db.database)
        .env("SURREAL_USER", &db.username)
        .env("SURREAL_PASS", &db.password)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to start MCP proxy server. Is Node.js installed?")?;

    let status = child.wait().context("MCP proxy server exited unexpectedly")?;

    if !status.success() {
        bail!("MCP proxy server exited with status: {}", status);
    }

    Ok(())
}
