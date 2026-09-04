use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::backend::{self, ResolvedSurrealDb, DEFAULT_CONFIG_PATH};

/// Resolve the path to the devtools-mcp entry point.
fn resolve_devtools_mcp() -> Result<PathBuf> {
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().unwrap_or(Path::new("."));

    let candidates = [
        // npx: binary at node_modules/@spooky-sync/cli-darwin-arm64/spky
        //       devtools-mcp at node_modules/@spooky-sync/cli/devtools-mcp/
        exe_dir.join("../../cli/devtools-mcp/index.js"),
        // Bundled: devtools-mcp/ sits next to the binary
        exe_dir.join("../devtools-mcp/index.js"),
        // Dev monorepo: relative to CWD
        PathBuf::from("apps/cli/devtools-mcp/index.js"),
        // Dev monorepo: binary at apps/cli/target/release/spky
        exe_dir.join("../../../devtools-mcp/dist/index.js"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone()));
        }
    }

    // Fall back to node's own resolver, run from the CWD rather than from the
    // binary. Under pnpm the platform binary lives in its own .pnpm directory
    // with no @spooky-sync/cli sibling, so none of the relative candidates
    // above can hit; the consumer's node_modules still has the symlink.
    // The server ships inside @spooky-sync/cli, so ask for that subpath first.
    for specifier in ["@spooky-sync/cli/devtools-mcp", "@spooky-sync/devtools-mcp"] {
        let output = Command::new("node")
            .args([
                "-e",
                &format!("console.log(require.resolve('{specifier}'))"),
            ])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
    }

    bail!(
        "Could not find @spooky-sync/devtools-mcp. Checked:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n  \
         - {}\n  \
         - node require.resolve('@spooky-sync/cli/devtools-mcp')\n\
         - node require.resolve('@spooky-sync/devtools-mcp')\n\
         Try reinstalling @spooky-sync/cli; the MCP server ships inside it.",
        candidates[0].display(),
        candidates[1].display(),
        candidates[2].display(),
        candidates[3].display(),
    )
}

pub fn run() -> Result<()> {
    let mcp_path = resolve_devtools_mcp().context("Failed to locate MCP server")?;

    // Load sp00ky.yml for SurrealDB connection defaults
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let db = ResolvedSurrealDb::from_config(&config.surrealdb);

    // In dev mode, SurrealDB runs on port 8666
    let surreal_url =
        std::env::var("SURREAL_URL").unwrap_or_else(|_| format!("http://localhost:8666"));

    let mut child = Command::new("node")
        .arg(&mcp_path)
        .env("SURREAL_URL", &surreal_url)
        .env("SURREAL_NS", &db.namespace)
        .env("SURREAL_DB", &db.database)
        .env("SURREAL_USER", db.username_literal())
        .env("SURREAL_PASS", db.password_literal())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to start MCP server. Is Node.js installed?")?;

    let status = child.wait().context("MCP server exited unexpectedly")?;

    if !status.success() {
        bail!("MCP server exited with status: {}", status);
    }

    Ok(())
}
