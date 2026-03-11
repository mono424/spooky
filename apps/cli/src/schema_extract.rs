use anyhow::{bail, Context, Result};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use crate::migrate;
use crate::surreal_client::SurrealClient;

/// Extract the full schema from a running SurrealDB as DEFINE statements.
///
/// 1. INFO FOR DB → tables, functions, accesses, analyzers, params
/// 2. INFO FOR TABLE <name> for each table → fields, indexes, events
/// Returns a string of all DEFINE statements joined with ";\n".
pub fn extract_schema_from_db(client: &SurrealClient) -> Result<String> {
    let db_info = client.info_for_db()?;
    let mut statements = Vec::new();

    // Extract top-level objects from INFO FOR DB
    // The response has keys like "tables", "functions", "accesses", "analyzers", "params"
    if let Some(obj) = db_info.as_object() {
        // Collect table names for subsequent INFO FOR TABLE calls
        let mut table_names = Vec::new();

        for (section, values) in obj {
            if let Some(inner_obj) = values.as_object() {
                for (name, define_stmt) in inner_obj {
                    if let Some(stmt_str) = define_stmt.as_str() {
                        statements.push(ensure_semicolon(stmt_str));
                    }
                    if section == "tables" {
                        table_names.push(name.clone());
                    }
                }
            }
        }

        // For each table, get detailed info (fields, indexes, events)
        for table_name in &table_names {
            let table_info = client.info_for_table(table_name)?;
            if let Some(table_obj) = table_info.as_object() {
                for (_section, values) in table_obj {
                    if let Some(inner_obj) = values.as_object() {
                        for (_name, define_stmt) in inner_obj {
                            if let Some(stmt_str) = define_stmt.as_str() {
                                statements.push(ensure_semicolon(stmt_str));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(statements.join("\n"))
}

/// Spin up an ephemeral in-memory SurrealDB, apply existing migrations, then extract the schema.
pub fn extract_schema_via_ephemeral_db(migrations_dir: &Path) -> Result<String> {
    // Find a free port
    let port = find_free_port()?;

    // Start ephemeral SurrealDB
    let mut child = start_ephemeral_surreal(port)?;

    // Ensure we clean up the child process
    let result = (|| -> Result<String> {
        // Wait for health
        wait_for_surreal_health(port)?;

        // Create client
        let client = SurrealClient::new(
            &format!("http://127.0.0.1:{}", port),
            "main",
            "main",
            "root",
            "root",
        );

        // Ensure namespace and database exist
        client.ensure_ns_db()?;

        // Apply existing migrations
        if migrations_dir.exists() {
            migrate::apply(&client, migrations_dir)?;
        }

        // Extract schema
        extract_schema_from_db(&client)
    })();

    // Always kill the child process
    let _ = child.kill();
    let _ = child.wait();

    result
}

fn find_free_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("Failed to bind to find a free port")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn start_ephemeral_surreal(port: u16) -> Result<Child> {
    let child = Command::new("surreal")
        .args([
            "start",
            "memory",
            "--bind",
            &format!("127.0.0.1:{}", port),
            "--user",
            "root",
            "--pass",
            "root",
            "--allow-all",
            "--no-banner",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context(
            "Failed to start SurrealDB. Is the 'surreal' binary installed?\n\
             Install it from https://surrealdb.com/install or provide --url to use a live database.",
        )?;

    Ok(child)
}

fn wait_for_surreal_health(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/health", port);
    let max_retries = 30;
    let interval = Duration::from_millis(200);

    for attempt in 1..=max_retries {
        match ureq::get(&url).timeout(Duration::from_secs(2)).call() {
            Ok(resp) if resp.status() == 200 => return Ok(()),
            _ => {
                if attempt == max_retries {
                    bail!(
                        "Ephemeral SurrealDB did not become ready after {} attempts",
                        max_retries
                    );
                }
                thread::sleep(interval);
            }
        }
    }

    unreachable!()
}

fn ensure_semicolon(stmt: &str) -> String {
    let trimmed = stmt.trim();
    if trimmed.ends_with(';') {
        trimmed.to_string()
    } else {
        format!("{};", trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_semicolon_adds_when_missing() {
        assert_eq!(ensure_semicolon("DEFINE TABLE user"), "DEFINE TABLE user;");
    }

    #[test]
    fn test_ensure_semicolon_keeps_existing() {
        assert_eq!(
            ensure_semicolon("DEFINE TABLE user;"),
            "DEFINE TABLE user;"
        );
    }

    #[test]
    fn test_ensure_semicolon_trims_whitespace() {
        assert_eq!(
            ensure_semicolon("  DEFINE TABLE user  "),
            "DEFINE TABLE user;"
        );
    }

    #[test]
    fn test_find_free_port() {
        let port = find_free_port().unwrap();
        assert!(port > 0);
    }
}
