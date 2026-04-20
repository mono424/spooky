use anyhow::Result;

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

/// Generic migration engine trait.
///
/// Each method maps to a user-facing CLI command. Implementations
/// encapsulate all tool-specific details (file formats, protocols,
/// state tracking). No raw SQL or file paths leak through this trait.
pub trait MigrationEngine {
    /// Apply all pending migrations.
    fn apply(&self) -> Result<()>;

    /// Return the status of all known migrations.
    fn status(&self) -> Result<Vec<MigrationInfo>>;

    /// Fix drift: update checksums and/or generate corrective migrations.
    fn fix(&self, fix_checksums: bool) -> Result<()>;
}
