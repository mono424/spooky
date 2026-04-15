use anyhow::Result;
use std::path::PathBuf;

/// The environment a migration engine operates in.
/// Determines strategy (e.g., surrealkit uses sync for dev, rollout for prod).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MigrationEnvironment {
    Dev,
    Production,
}

/// Status of a single migration unit.
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationState {
    Applied,
    Pending,
    Drift,
}

/// Information about a single migration.
#[derive(Debug, Clone)]
pub struct MigrationInfo {
    pub id: String,
    pub name: String,
    pub state: MigrationState,
    pub applied_at: Option<String>,
    pub detail: Option<String>,
}

/// Result from an apply operation.
#[derive(Debug)]
pub struct ApplyResult {
    pub applied_count: usize,
    pub messages: Vec<String>,
}

/// Result from a create operation.
#[derive(Debug)]
pub struct CreateResult {
    pub file_path: Option<PathBuf>,
    pub message: String,
    pub has_changes: bool,
}

/// Generic migration engine trait.
///
/// Each method maps to a user-facing CLI command. Implementations
/// encapsulate all tool-specific details (file formats, protocols,
/// state tracking). No raw SQL or file paths leak through this trait.
pub trait MigrationEngine {
    /// Verify connectivity to the migration backend.
    fn check_connection(&self) -> Result<()>;

    /// Apply all pending migrations.
    fn apply(&self) -> Result<ApplyResult>;

    /// Return the status of all known migrations.
    fn status(&self) -> Result<Vec<MigrationInfo>>;

    /// Create a new migration from the current schema diff.
    fn create(&self, name: &str) -> Result<CreateResult>;

    /// Fix drift: update checksums and/or generate corrective migrations.
    fn fix(&self, fix_checksums: bool) -> Result<()>;
}
