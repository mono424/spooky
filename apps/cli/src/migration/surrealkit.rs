use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use super::engine::{
    ApplyResult, CreateResult, MigrationEngine, MigrationEnvironment, MigrationInfo,
    MigrationState,
};

/// SurrealKit migration engine that shells out to the `surrealkit` CLI binary.
///
/// In dev mode, uses `surrealkit sync` (fast, declarative push).
/// In production, uses `surrealkit rollout` (controlled, phased migrations).
pub(crate) struct SurrealKitEngine {
    binary: String,
    environment: MigrationEnvironment,
    project_dir: PathBuf,
    host: String,
    namespace: String,
    database: String,
    username: String,
    password: String,
}

impl SurrealKitEngine {
    pub(crate) fn new(
        binary: String,
        environment: MigrationEnvironment,
        project_dir: PathBuf,
        host: String,
        namespace: String,
        database: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        // Verify the binary exists and is executable
        let check = Command::new(&binary).arg("--version").output();
        match check {
            Ok(output) if output.status.success() => {}
            _ => {
                bail!(
                    "surrealkit binary not found at '{}'.\n\
                     Install it with `cargo install surrealkit` or set the path in sp00ky.yml:\n\
                     \n\
                     surrealkit:\n\
                     \x20 binary: /path/to/surrealkit",
                    binary
                );
            }
        }

        Ok(Self {
            binary,
            environment,
            project_dir,
            host,
            namespace,
            database,
            username,
            password,
        })
    }

    /// Build a Command with surrealkit env vars and working directory set.
    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.args(args);
        cmd.current_dir(&self.project_dir);
        cmd.env("SURREALDB_HOST", &self.host);
        cmd.env("SURREALDB_NAME", &self.database);
        cmd.env("SURREALDB_NAMESPACE", &self.namespace);
        cmd.env("SURREALDB_USERNAME", &self.username);
        cmd.env("SURREALDB_PASSWORD", &self.password);
        cmd
    }

    /// Run a surrealkit command, returning stdout on success.
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self
            .cmd(args)
            .output()
            .with_context(|| format!("Failed to run: {} {}", self.binary, args.join(" ")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            bail!(
                "{} {} failed:\n{}\n{}",
                self.binary,
                args.join(" "),
                stdout,
                stderr
            );
        }

        if !stderr.is_empty() {
            eprint!("{}", stderr);
        }

        Ok(stdout)
    }
}

impl MigrationEngine for SurrealKitEngine {
    fn check_connection(&self) -> Result<()> {
        self.run(&["rollout", "status"]).map(|_| ())
    }

    fn apply(&self) -> Result<ApplyResult> {
        match self.environment {
            MigrationEnvironment::Dev => {
                let output = self.run(&["sync"])?;
                Ok(ApplyResult {
                    applied_count: 0,
                    messages: vec![output],
                })
            }
            MigrationEnvironment::Production => {
                let start_output = self.run(&["rollout", "start"])?;
                let complete_output = self.run(&["rollout", "complete"])?;
                Ok(ApplyResult {
                    applied_count: 1,
                    messages: vec![start_output, complete_output],
                })
            }
        }
    }

    fn status(&self) -> Result<Vec<MigrationInfo>> {
        let output = self.run(&["rollout", "status"])?;
        // surrealkit outputs human-readable text; wrap it as a single status entry
        let state = if output.contains("up to date") || output.contains("No pending") {
            MigrationState::Applied
        } else {
            MigrationState::Pending
        };
        Ok(vec![MigrationInfo {
            id: "surrealkit".to_string(),
            name: "rollout status".to_string(),
            state,
            applied_at: None,
            detail: Some(output),
        }])
    }

    fn create(&self, _name: &str) -> Result<CreateResult> {
        match self.environment {
            MigrationEnvironment::Dev => Ok(CreateResult {
                file_path: None,
                message: "Dev mode uses declarative sync -- no migration file needed.".to_string(),
                has_changes: false,
            }),
            MigrationEnvironment::Production => {
                let output = self.run(&["rollout", "plan"])?;
                Ok(CreateResult {
                    file_path: None,
                    message: output,
                    has_changes: true,
                })
            }
        }
    }

    fn fix(&self, _fix_checksums: bool) -> Result<()> {
        match self.environment {
            MigrationEnvironment::Dev => {
                // Sync mode is inherently self-fixing
                self.run(&["sync"])?;
                Ok(())
            }
            MigrationEnvironment::Production => {
                let _ = self.run(&["rollout", "rollback"]);
                self.run(&["rollout", "plan"])?;
                Ok(())
            }
        }
    }
}
