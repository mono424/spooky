use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use crate::surreal_client::MigrationDB;

/// A migration discovered on the filesystem.
pub(crate) struct FilesystemMigration {
    pub version: String,
    pub name: String,
    #[allow(dead_code)]
    pub dir_path: PathBuf,
    pub up_path: PathBuf,
    pub down_path: PathBuf,
}

/// Scan the migrations directory and return sorted migrations.
pub(crate) fn scan_migrations(migrations_dir: &Path) -> Result<Vec<FilesystemMigration>> {
    if !migrations_dir.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    for entry in fs::read_dir(migrations_dir).context("Failed to read migrations directory")? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Parse "{version}_{name}" pattern
        let underscore_pos = match dir_name.find('_') {
            Some(pos) => pos,
            None => continue,
        };

        let version = dir_name[..underscore_pos].to_string();
        let name = dir_name[underscore_pos + 1..].to_string();

        // Validate version looks like a timestamp
        if version.len() != 14 || !version.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        migrations.push(FilesystemMigration {
            version,
            name,
            dir_path: path.clone(),
            up_path: path.join("up.surql"),
            down_path: path.join("down.surql"),
        });
    }

    migrations.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(migrations)
}

/// Compute SHA-256 checksum of a file's contents.
pub(crate) fn checksum(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .context(format!("Failed to read file for checksum: {:?}", path))?;
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// Sanitize a migration name: lowercase, replace spaces/hyphens with underscores.
pub(crate) fn sanitize_name(name: &str) -> String {
    name.to_lowercase()
        .replace([' ', '-'], "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

// ── Commands ────────────────────────────────────────────────────────────────

/// Create a new migration directory with stub files.
pub fn create(migrations_dir: &Path, name: &str, schema_path: Option<&Path>) -> Result<()> {
    let sanitized = sanitize_name(name);
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let dir_name = format!("{}_{}", timestamp, sanitized);
    let dir_path = migrations_dir.join(&dir_name);

    fs::create_dir_all(&dir_path)
        .context(format!("Failed to create migration directory: {:?}", dir_path))?;

    // up.surql: start with schema content if provided, otherwise empty stub
    let up_content = if let Some(schema) = schema_path {
        let schema_content = fs::read_to_string(schema)
            .context(format!("Failed to read schema file: {:?}", schema))?;
        format!(
            "-- Migration: {}\n-- Created: {}\n--\n-- WARNING: This file was pre-populated with the full schema.\n-- Edit this to contain ONLY the incremental changes for this migration.\n\n{}",
            sanitized,
            chrono::Utc::now().to_rfc3339(),
            schema_content
        )
    } else {
        format!(
            "-- Migration: {}\n-- Created: {}\n--\n-- Write your forward migration SurrealQL here.\n",
            sanitized,
            chrono::Utc::now().to_rfc3339()
        )
    };

    let down_content = format!(
        "-- Rollback: {}\n-- Created: {}\n--\n-- Write your rollback SurrealQL here.\n-- This should undo everything in up.surql.\n",
        sanitized,
        chrono::Utc::now().to_rfc3339()
    );

    fs::write(dir_path.join("up.surql"), up_content)
        .context("Failed to write up.surql")?;
    fs::write(dir_path.join("down.surql"), down_content)
        .context("Failed to write down.surql")?;

    println!("Created migration: {}", dir_name);
    println!("  {}/up.surql", dir_path.display());
    println!("  {}/down.surql", dir_path.display());

    Ok(())
}

/// Apply all pending migrations in order.
pub fn apply(client: &dyn MigrationDB, migrations_dir: &Path) -> Result<()> {
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let applied = client.get_applied_migrations()?;
    let filesystem = scan_migrations(migrations_dir)?;

    // Integrity check: verify checksums of applied migrations
    for am in &applied {
        if let Some(fm) = filesystem.iter().find(|f| f.version == am.version) {
            if !fm.up_path.exists() {
                println!(
                    "WARNING: Applied migration {}_{} is missing from disk",
                    am.version, am.name
                );
                continue;
            }
            let disk_checksum = checksum(&fm.up_path)?;
            if disk_checksum != am.checksum {
                bail!(
                    "Checksum mismatch for migration {}_{}\n  Expected: {}\n  Found:    {}\n\nThe migration file has been modified after it was applied. This is dangerous.\nIf intentional, manually update the checksum in _spooky_migrations.",
                    am.version,
                    am.name,
                    am.checksum,
                    disk_checksum
                );
            }
        } else {
            println!(
                "WARNING: Applied migration {}_{} not found on disk (drift detected)",
                am.version, am.name
            );
        }
    }

    // Find pending migrations
    let applied_versions: Vec<&str> = applied.iter().map(|a| a.version.as_str()).collect();
    let pending: Vec<&FilesystemMigration> = filesystem
        .iter()
        .filter(|f| !applied_versions.contains(&f.version.as_str()))
        .collect();

    if pending.is_empty() {
        println!("No pending migrations.");
        return Ok(());
    }

    println!("Applying {} migration(s)...\n", pending.len());

    for migration in &pending {
        if !migration.up_path.exists() {
            bail!(
                "Missing up.surql for migration {}_{}",
                migration.version,
                migration.name
            );
        }

        let sql = fs::read_to_string(&migration.up_path)
            .context(format!("Failed to read {:?}", migration.up_path))?;

        let hash = checksum(&migration.up_path)?;

        println!(
            "  Applying {}_{} ...",
            migration.version, migration.name
        );

        client
            .execute(&sql)
            .context(format!(
                "Failed to apply migration {}_{}",
                migration.version, migration.name
            ))?;

        client.record_migration(&migration.version, &migration.name, &hash)?;

        println!(
            "  Applied  {}_{} [ok]",
            migration.version, migration.name
        );
    }

    println!("\nAll migrations applied successfully.");
    Ok(())
}

/// Rollback the last N applied migrations.
pub fn rollback(client: &dyn MigrationDB, migrations_dir: &Path, steps: usize) -> Result<()> {
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let applied = client.get_applied_migrations()?;
    let filesystem = scan_migrations(migrations_dir)?;

    if applied.is_empty() {
        println!("No applied migrations to rollback.");
        return Ok(());
    }

    let to_rollback: Vec<_> = applied.iter().rev().take(steps).collect();

    println!("Rolling back {} migration(s)...\n", to_rollback.len());

    for am in &to_rollback {
        let fm = filesystem
            .iter()
            .find(|f| f.version == am.version)
            .context(format!(
                "Migration {}_{} not found on disk — cannot rollback",
                am.version, am.name
            ))?;

        if !fm.down_path.exists() {
            bail!(
                "Missing down.surql for migration {}_{}.\nCannot rollback without a down migration.",
                am.version,
                am.name
            );
        }

        let sql = fs::read_to_string(&fm.down_path)
            .context(format!("Failed to read {:?}", fm.down_path))?;

        let trimmed = sql.lines()
            .filter(|l| !l.starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");

        if trimmed.trim().is_empty() {
            bail!(
                "down.surql for migration {}_{} is empty (only comments).\nWrite rollback SQL before attempting rollback.",
                am.version,
                am.name
            );
        }

        println!(
            "  Rolling back {}_{} ...",
            am.version, am.name
        );

        client
            .execute(&sql)
            .context(format!(
                "Failed to rollback migration {}_{}",
                am.version, am.name
            ))?;

        client.remove_migration(&am.version)?;

        println!(
            "  Rolled back  {}_{} [ok]",
            am.version, am.name
        );
    }

    println!("\nRollback completed.");
    Ok(())
}

/// Display migration status.
pub fn status(client: &dyn MigrationDB, migrations_dir: &Path) -> Result<()> {
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let applied = client.get_applied_migrations()?;
    let filesystem = scan_migrations(migrations_dir)?;

    if filesystem.is_empty() && applied.is_empty() {
        println!("No migrations found.");
        return Ok(());
    }

    println!("Migration Status:\n");

    // Show filesystem migrations with their status
    for fm in &filesystem {
        if let Some(am) = applied.iter().find(|a| a.version == fm.version) {
            // Check for checksum mismatch
            if fm.up_path.exists() {
                let disk_checksum = checksum(&fm.up_path)?;
                if disk_checksum != am.checksum {
                    println!(
                        "  [DRIFT]    {}_{:<40} (checksum mismatch!)",
                        fm.version, fm.name
                    );
                    continue;
                }
            }
            println!(
                "  [applied]  {}_{:<40} (applied {})",
                fm.version, fm.name, am.applied_at
            );
        } else {
            println!("  [pending]  {}_{}", fm.version, fm.name);
        }
    }

    // Warn about applied migrations not on disk
    for am in &applied {
        if !filesystem.iter().any(|f| f.version == am.version) {
            println!(
                "\n  WARNING: Applied migration {}_{} is not present on disk (drift)",
                am.version, am.name
            );
        }
    }

    println!();
    Ok(())
}

/// Reset: rollback all applied migrations.
pub fn reset(client: &dyn MigrationDB, migrations_dir: &Path) -> Result<()> {
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let applied = client.get_applied_migrations()?;
    let count = applied.len();

    if count == 0 {
        println!("No applied migrations to reset.");
        return Ok(());
    }

    println!("Resetting all {} migration(s)...", count);
    rollback(client, migrations_dir, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surreal_client::{AppliedMigration, MigrationDB, SurrealResponse};
    use std::cell::RefCell;
    use tempfile::TempDir;

    // ── Mock DB ─────────────────────────────────────────────────────────

    /// A mock MigrationDB that records calls and returns configurable results.
    struct MockDB {
        applied: RefCell<Vec<AppliedMigration>>,
        executed_queries: RefCell<Vec<String>>,
        recorded: RefCell<Vec<(String, String, String)>>,
        removed: RefCell<Vec<String>>,
        fail_execute: RefCell<bool>,
    }

    impl MockDB {
        fn new() -> Self {
            Self {
                applied: RefCell::new(vec![]),
                executed_queries: RefCell::new(vec![]),
                recorded: RefCell::new(vec![]),
                removed: RefCell::new(vec![]),
                fail_execute: RefCell::new(false),
            }
        }

        fn with_applied(migrations: Vec<AppliedMigration>) -> Self {
            let mock = Self::new();
            *mock.applied.borrow_mut() = migrations;
            mock
        }

        fn set_fail_execute(&self, fail: bool) {
            *self.fail_execute.borrow_mut() = fail;
        }
    }

    impl MigrationDB for MockDB {
        fn ping(&self) -> Result<()> {
            Ok(())
        }

        fn ensure_migration_table(&self) -> Result<()> {
            Ok(())
        }

        fn execute(&self, query: &str) -> Result<Vec<SurrealResponse>> {
            if *self.fail_execute.borrow() {
                anyhow::bail!("Mock execute failure");
            }
            self.executed_queries.borrow_mut().push(query.to_string());
            Ok(vec![SurrealResponse {
                status: "OK".to_string(),
                result: None,
            }])
        }

        fn get_applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
            Ok(self.applied.borrow().clone())
        }

        fn record_migration(&self, version: &str, name: &str, checksum: &str) -> Result<()> {
            self.recorded.borrow_mut().push((
                version.to_string(),
                name.to_string(),
                checksum.to_string(),
            ));
            Ok(())
        }

        fn remove_migration(&self, version: &str) -> Result<()> {
            self.removed.borrow_mut().push(version.to_string());
            Ok(())
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Create a migration directory with up.surql and optional down.surql content.
    fn create_migration_dir(
        base: &Path,
        version: &str,
        name: &str,
        up_content: &str,
        down_content: Option<&str>,
    ) -> PathBuf {
        let dir = base.join(format!("{}_{}", version, name));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("up.surql"), up_content).unwrap();
        if let Some(down) = down_content {
            fs::write(dir.join("down.surql"), down).unwrap();
        }
        dir
    }

    fn make_applied(version: &str, name: &str, chksum: &str) -> AppliedMigration {
        AppliedMigration {
            version: version.to_string(),
            name: name.to_string(),
            applied_at: "2024-01-01T12:00:00Z".to_string(),
            checksum: chksum.to_string(),
        }
    }

    /// Compute the checksum of a string (matching what checksum() does for file contents).
    fn checksum_str(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    // ── sanitize_name tests ─────────────────────────────────────────────

    #[test]
    fn test_sanitize_name_simple() {
        assert_eq!(sanitize_name("initial"), "initial");
    }

    #[test]
    fn test_sanitize_name_spaces_to_underscores() {
        assert_eq!(sanitize_name("add user table"), "add_user_table");
    }

    #[test]
    fn test_sanitize_name_hyphens_to_underscores() {
        assert_eq!(sanitize_name("add-user-table"), "add_user_table");
    }

    #[test]
    fn test_sanitize_name_uppercase_to_lowercase() {
        assert_eq!(sanitize_name("AddUserTable"), "addusertable");
    }

    #[test]
    fn test_sanitize_name_strips_special_chars() {
        assert_eq!(sanitize_name("add@user#table!"), "addusertable");
    }

    #[test]
    fn test_sanitize_name_mixed_transforms() {
        assert_eq!(
            sanitize_name("Add User-Avatar v2!"),
            "add_user_avatar_v2"
        );
    }

    #[test]
    fn test_sanitize_name_empty_string() {
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn test_sanitize_name_only_special_chars() {
        assert_eq!(sanitize_name("@#$%"), "");
    }

    // ── checksum tests ──────────────────────────────────────────────────

    #[test]
    fn test_checksum_consistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.surql");
        fs::write(&path, "DEFINE TABLE test;").unwrap();

        let hash1 = checksum(&path).unwrap();
        let hash2 = checksum(&path).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_checksum_different_content_differs() {
        let dir = TempDir::new().unwrap();
        let path1 = dir.path().join("a.surql");
        let path2 = dir.path().join("b.surql");
        fs::write(&path1, "DEFINE TABLE a;").unwrap();
        fs::write(&path2, "DEFINE TABLE b;").unwrap();

        assert_ne!(checksum(&path1).unwrap(), checksum(&path2).unwrap());
    }

    #[test]
    fn test_checksum_missing_file_errors() {
        let result = checksum(Path::new("/nonexistent/file.surql"));
        assert!(result.is_err());
    }

    #[test]
    fn test_checksum_is_valid_hex_sha256() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.surql");
        fs::write(&path, "content").unwrap();

        let hash = checksum(&path).unwrap();
        // SHA-256 hex output is 64 chars
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── scan_migrations tests ───────────────────────────────────────────

    #[test]
    fn test_scan_nonexistent_dir_returns_empty() {
        let result = scan_migrations(Path::new("/nonexistent/migrations")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_empty_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_finds_valid_migration() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "initial", "-- up", Some("-- down"));

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "20240101120000");
        assert_eq!(result[0].name, "initial");
    }

    #[test]
    fn test_scan_sorts_by_version() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240103120000", "third", "-- up", None);
        create_migration_dir(dir.path(), "20240101120000", "first", "-- up", None);
        create_migration_dir(dir.path(), "20240102120000", "second", "-- up", None);

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].version, "20240101120000");
        assert_eq!(result[1].version, "20240102120000");
        assert_eq!(result[2].version, "20240103120000");
    }

    #[test]
    fn test_scan_skips_non_directory_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("20240101120000_not_a_dir"), "file").unwrap();
        create_migration_dir(dir.path(), "20240101120000", "real", "-- up", None);

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "real");
    }

    #[test]
    fn test_scan_skips_dirs_without_underscore() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("nounderscore")).unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_skips_dirs_with_short_version() {
        let dir = TempDir::new().unwrap();
        // Version must be exactly 14 digits
        fs::create_dir_all(dir.path().join("2024_too_short")).unwrap();
        fs::create_dir_all(dir.path().join("12345_short")).unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_skips_dirs_with_non_digit_version() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("2024010112000a_bad_ver")).unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_sets_correct_paths() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "test", "-- up", Some("-- down"));

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result[0].up_path, dir.path().join("20240101120000_test/up.surql"));
        assert_eq!(result[0].down_path, dir.path().join("20240101120000_test/down.surql"));
    }

    #[test]
    fn test_scan_handles_multi_underscore_names() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "add_user_table", "-- up", None);

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        // The name should be everything after the first underscore
        assert_eq!(result[0].name, "add_user_table");
    }

    // ── create tests ────────────────────────────────────────────────────

    #[test]
    fn test_create_makes_directory_and_files() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "initial", None).unwrap();

        // Should have created one subdirectory
        let entries: Vec<_> = fs::read_dir(&migrations_dir).unwrap().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        assert!(entry.path().join("up.surql").exists());
        assert!(entry.path().join("down.surql").exists());
    }

    #[test]
    fn test_create_uses_timestamp_name_pattern() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "add_users", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let name = entry.file_name().to_string_lossy().to_string();

        // Format: {14-digit timestamp}_{sanitized_name}
        assert!(name.ends_with("_add_users"));
        let version = &name[..14];
        assert!(version.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(version.len(), 14);
    }

    #[test]
    fn test_create_sanitizes_name() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "Add User-Avatar!", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(name.ends_with("_add_user_avatar"));
    }

    #[test]
    fn test_create_up_surql_has_header_comment() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "test", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let up_content = fs::read_to_string(entry.path().join("up.surql")).unwrap();
        assert!(up_content.starts_with("-- Migration: test"));
        assert!(up_content.contains("-- Created:"));
        assert!(up_content.contains("Write your forward migration"));
    }

    #[test]
    fn test_create_down_surql_has_header_comment() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "test", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let down_content = fs::read_to_string(entry.path().join("down.surql")).unwrap();
        assert!(down_content.starts_with("-- Rollback: test"));
        assert!(down_content.contains("undo everything in up.surql"));
    }

    #[test]
    fn test_create_with_schema_prepopulates_up() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        // Create a schema file
        let schema_path = dir.path().join("schema.surql");
        fs::write(&schema_path, "DEFINE TABLE user SCHEMAFULL;").unwrap();

        create(&migrations_dir, "initial", Some(&schema_path)).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let up_content = fs::read_to_string(entry.path().join("up.surql")).unwrap();
        assert!(up_content.contains("DEFINE TABLE user SCHEMAFULL;"));
        assert!(up_content.contains("WARNING"));
        assert!(up_content.contains("ONLY the incremental changes"));
    }

    #[test]
    fn test_create_with_nonexistent_schema_errors() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        let result = create(
            &migrations_dir,
            "test",
            Some(Path::new("/nonexistent/schema.surql")),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_create_multiple_migrations_are_unique() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "first", None).unwrap();
        // Sleep a tiny bit so timestamp differs (or they'll collide)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        create(&migrations_dir, "second", None).unwrap();

        let entries: Vec<_> = fs::read_dir(&migrations_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_create_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("deep/nested/migrations");

        create(&migrations_dir, "test", None).unwrap();
        assert!(migrations_dir.exists());
    }

    // ── apply tests ─────────────────────────────────────────────────────

    #[test]
    fn test_apply_no_pending_migrations() {
        let dir = TempDir::new().unwrap();
        let mock = MockDB::new();

        // Empty dir = no migrations
        apply(&mock, dir.path()).unwrap();
        assert!(mock.recorded.borrow().is_empty());
    }

    #[test]
    fn test_apply_applies_single_pending_migration() {
        let dir = TempDir::new().unwrap();
        let up_sql = "DEFINE TABLE user SCHEMAFULL;";
        create_migration_dir(dir.path(), "20240101120000", "initial", up_sql, Some("-- down"));

        let mock = MockDB::new();
        apply(&mock, dir.path()).unwrap();

        let recorded = mock.recorded.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "20240101120000");
        assert_eq!(recorded[0].1, "initial");
        assert_eq!(recorded[0].2, checksum_str(up_sql));
    }

    #[test]
    fn test_apply_applies_multiple_in_order() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "first", "CREATE first;", Some("--"));
        create_migration_dir(dir.path(), "20240102120000", "second", "CREATE second;", Some("--"));
        create_migration_dir(dir.path(), "20240103120000", "third", "CREATE third;", Some("--"));

        let mock = MockDB::new();
        apply(&mock, dir.path()).unwrap();

        let recorded = mock.recorded.borrow();
        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[0].0, "20240101120000");
        assert_eq!(recorded[1].0, "20240102120000");
        assert_eq!(recorded[2].0, "20240103120000");

        // Verify the SQL was actually executed
        let queries = mock.executed_queries.borrow();
        assert_eq!(queries.len(), 3);
        assert_eq!(queries[0], "CREATE first;");
        assert_eq!(queries[1], "CREATE second;");
        assert_eq!(queries[2], "CREATE third;");
    }

    #[test]
    fn test_apply_skips_already_applied() {
        let dir = TempDir::new().unwrap();
        let up_sql = "CREATE first;";
        create_migration_dir(dir.path(), "20240101120000", "first", up_sql, Some("--"));
        create_migration_dir(dir.path(), "20240102120000", "second", "CREATE second;", Some("--"));

        // Mark first as already applied
        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "first",
            &checksum_str(up_sql),
        )]);

        apply(&mock, dir.path()).unwrap();

        let recorded = mock.recorded.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "20240102120000");
    }

    #[test]
    fn test_apply_checksum_mismatch_aborts() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(
            dir.path(),
            "20240101120000",
            "initial",
            "MODIFIED content",
            Some("--"),
        );

        // Applied migration has a different checksum (original content was different)
        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "initial",
            "original_checksum_that_doesnt_match",
        )]);

        let result = apply(&mock, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Checksum mismatch"));
    }

    #[test]
    fn test_apply_missing_up_surql_aborts() {
        let dir = TempDir::new().unwrap();
        // Create directory but no up.surql
        let migration_dir = dir.path().join("20240101120000_broken");
        fs::create_dir_all(&migration_dir).unwrap();

        let mock = MockDB::new();
        let result = apply(&mock, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing up.surql"));
    }

    #[test]
    fn test_apply_execute_failure_aborts() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(
            dir.path(),
            "20240101120000",
            "initial",
            "INVALID SQL;",
            Some("--"),
        );

        let mock = MockDB::new();
        mock.set_fail_execute(true);

        let result = apply(&mock, dir.path());
        assert!(result.is_err());
        // No migration should have been recorded
        assert!(mock.recorded.borrow().is_empty());
    }

    #[test]
    fn test_apply_all_already_applied() {
        let dir = TempDir::new().unwrap();
        let up_sql = "CREATE stuff;";
        create_migration_dir(dir.path(), "20240101120000", "initial", up_sql, Some("--"));

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "initial",
            &checksum_str(up_sql),
        )]);

        apply(&mock, dir.path()).unwrap();
        assert!(mock.recorded.borrow().is_empty());
    }

    // ── rollback tests ──────────────────────────────────────────────────

    #[test]
    fn test_rollback_no_applied_migrations() {
        let dir = TempDir::new().unwrap();
        let mock = MockDB::new();

        rollback(&mock, dir.path(), 1).unwrap();
        assert!(mock.removed.borrow().is_empty());
    }

    #[test]
    fn test_rollback_single_step() {
        let dir = TempDir::new().unwrap();
        let down_sql = "REMOVE TABLE user;";
        create_migration_dir(dir.path(), "20240101120000", "initial", "-- up", Some(down_sql));

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "initial",
            "somechecksum",
        )]);

        rollback(&mock, dir.path(), 1).unwrap();

        let removed = mock.removed.borrow();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "20240101120000");

        let queries = mock.executed_queries.borrow();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0], down_sql);
    }

    #[test]
    fn test_rollback_multiple_steps_in_reverse_order() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "first", "-- up", Some("DROP first;"));
        create_migration_dir(dir.path(), "20240102120000", "second", "-- up", Some("DROP second;"));
        create_migration_dir(dir.path(), "20240103120000", "third", "-- up", Some("DROP third;"));

        let mock = MockDB::with_applied(vec![
            make_applied("20240101120000", "first", "c1"),
            make_applied("20240102120000", "second", "c2"),
            make_applied("20240103120000", "third", "c3"),
        ]);

        rollback(&mock, dir.path(), 2).unwrap();

        let removed = mock.removed.borrow();
        assert_eq!(removed.len(), 2);
        // Should roll back in reverse order: third first, then second
        assert_eq!(removed[0], "20240103120000");
        assert_eq!(removed[1], "20240102120000");

        let queries = mock.executed_queries.borrow();
        assert_eq!(queries[0], "DROP third;");
        assert_eq!(queries[1], "DROP second;");
    }

    #[test]
    fn test_rollback_steps_exceeding_applied_count() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "only", "-- up", Some("DROP only;"));

        let mock = MockDB::with_applied(vec![make_applied("20240101120000", "only", "c1")]);

        // Request 5 steps but only 1 applied — should only roll back 1
        rollback(&mock, dir.path(), 5).unwrap();

        let removed = mock.removed.borrow();
        assert_eq!(removed.len(), 1);
    }

    #[test]
    fn test_rollback_missing_down_surql_errors() {
        let dir = TempDir::new().unwrap();
        // Create migration without down.surql
        let mig_dir = dir.path().join("20240101120000_broken");
        fs::create_dir_all(&mig_dir).unwrap();
        fs::write(mig_dir.join("up.surql"), "-- up").unwrap();
        // No down.surql

        let mock = MockDB::with_applied(vec![make_applied("20240101120000", "broken", "c1")]);

        let result = rollback(&mock, dir.path(), 1);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing down.surql"));
    }

    #[test]
    fn test_rollback_empty_down_surql_errors() {
        let dir = TempDir::new().unwrap();
        // down.surql with only comments
        create_migration_dir(
            dir.path(),
            "20240101120000",
            "empty_down",
            "-- up",
            Some("-- This is only a comment\n-- Another comment"),
        );

        let mock =
            MockDB::with_applied(vec![make_applied("20240101120000", "empty_down", "c1")]);

        let result = rollback(&mock, dir.path(), 1);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"));
    }

    #[test]
    fn test_rollback_migration_not_on_disk_errors() {
        let dir = TempDir::new().unwrap();
        // Applied migration exists but no directory on disk

        let mock = MockDB::with_applied(vec![make_applied("20240101120000", "ghost", "c1")]);

        let result = rollback(&mock, dir.path(), 1);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found on disk"));
    }

    #[test]
    fn test_rollback_execute_failure_aborts() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(
            dir.path(),
            "20240101120000",
            "fail",
            "-- up",
            Some("INVALID SQL;"),
        );

        let mock = MockDB::with_applied(vec![make_applied("20240101120000", "fail", "c1")]);
        mock.set_fail_execute(true);

        let result = rollback(&mock, dir.path(), 1);
        assert!(result.is_err());
        // Migration record should NOT have been removed since execute failed
        assert!(mock.removed.borrow().is_empty());
    }

    // ── status tests ────────────────────────────────────────────────────

    #[test]
    fn test_status_no_migrations() {
        let dir = TempDir::new().unwrap();
        let mock = MockDB::new();

        // Should succeed (just prints "No migrations found")
        status(&mock, dir.path()).unwrap();
    }

    #[test]
    fn test_status_with_pending_only() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "pending", "-- up", None);

        let mock = MockDB::new();
        // Should succeed without error
        status(&mock, dir.path()).unwrap();
    }

    #[test]
    fn test_status_with_applied_and_pending() {
        let dir = TempDir::new().unwrap();
        let up_sql = "CREATE first;";
        create_migration_dir(dir.path(), "20240101120000", "applied", up_sql, None);
        create_migration_dir(dir.path(), "20240102120000", "pending", "CREATE second;", None);

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "applied",
            &checksum_str(up_sql),
        )]);

        // Should succeed without error
        status(&mock, dir.path()).unwrap();
    }

    #[test]
    fn test_status_detects_checksum_drift() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(
            dir.path(),
            "20240101120000",
            "drifted",
            "MODIFIED content",
            None,
        );

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "drifted",
            "original_checksum_doesnt_match",
        )]);

        // Should succeed (drift is a warning, not an error in status)
        status(&mock, dir.path()).unwrap();
    }

    // ── reset tests ─────────────────────────────────────────────────────

    #[test]
    fn test_reset_no_applied_migrations() {
        let dir = TempDir::new().unwrap();
        let mock = MockDB::new();

        reset(&mock, dir.path()).unwrap();
        assert!(mock.removed.borrow().is_empty());
    }

    #[test]
    fn test_reset_rolls_back_all_applied() {
        let dir = TempDir::new().unwrap();
        create_migration_dir(dir.path(), "20240101120000", "first", "-- up", Some("DROP first;"));
        create_migration_dir(dir.path(), "20240102120000", "second", "-- up", Some("DROP second;"));

        let mock = MockDB::with_applied(vec![
            make_applied("20240101120000", "first", "c1"),
            make_applied("20240102120000", "second", "c2"),
        ]);

        reset(&mock, dir.path()).unwrap();

        let removed = mock.removed.borrow();
        assert_eq!(removed.len(), 2);
    }

    // ── Integration-style tests (filesystem + mock DB) ──────────────────

    #[test]
    fn test_full_lifecycle_create_apply_rollback() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        // Step 1: Create a migration
        create(&migrations_dir, "add_users", None).unwrap();

        // Write actual SQL to the created migration
        let entries: Vec<_> = fs::read_dir(&migrations_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);

        let mig_dir = &entries[0].path();
        let up_sql = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON user TYPE string;";
        let down_sql = "REMOVE TABLE user;";
        fs::write(mig_dir.join("up.surql"), up_sql).unwrap();
        fs::write(mig_dir.join("down.surql"), down_sql).unwrap();

        // Step 2: Apply
        let mock = MockDB::new();
        apply(&mock, &migrations_dir).unwrap();

        let recorded = mock.recorded.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, "add_users");

        let queries = mock.executed_queries.borrow();
        assert_eq!(queries[0], up_sql);
        drop(recorded);
        drop(queries);

        // Step 3: Re-apply should be no-op (all applied)
        let version = {
            let r = mock.recorded.borrow();
            r[0].0.clone()
        };
        let chk = {
            let r = mock.recorded.borrow();
            r[0].2.clone()
        };
        let mock2 = MockDB::with_applied(vec![make_applied(&version, "add_users", &chk)]);

        apply(&mock2, &migrations_dir).unwrap();
        assert!(mock2.recorded.borrow().is_empty());

        // Step 4: Rollback
        let mock3 = MockDB::with_applied(vec![make_applied(&version, "add_users", &chk)]);
        rollback(&mock3, &migrations_dir, 1).unwrap();

        let removed = mock3.removed.borrow();
        assert_eq!(removed.len(), 1);
        let queries = mock3.executed_queries.borrow();
        assert_eq!(queries[0], down_sql);
    }

    #[test]
    fn test_apply_then_create_new_then_apply_again() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        // First migration
        let up1 = "DEFINE TABLE user SCHEMAFULL;";
        fs::create_dir_all(migrations_dir.join("20240101120000_initial")).unwrap();
        fs::write(
            migrations_dir.join("20240101120000_initial/up.surql"),
            up1,
        )
        .unwrap();
        fs::write(
            migrations_dir.join("20240101120000_initial/down.surql"),
            "REMOVE TABLE user;",
        )
        .unwrap();

        // Apply first
        let mock = MockDB::new();
        apply(&mock, &migrations_dir).unwrap();
        assert_eq!(mock.recorded.borrow().len(), 1);

        // Add second migration
        let up2 = "DEFINE FIELD avatar ON user TYPE string;";
        fs::create_dir_all(migrations_dir.join("20240102120000_add_avatar")).unwrap();
        fs::write(
            migrations_dir.join("20240102120000_add_avatar/up.surql"),
            up2,
        )
        .unwrap();
        fs::write(
            migrations_dir.join("20240102120000_add_avatar/down.surql"),
            "REMOVE FIELD avatar ON user;",
        )
        .unwrap();

        // Apply again with first already applied
        let mock2 = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "initial",
            &checksum_str(up1),
        )]);
        apply(&mock2, &migrations_dir).unwrap();

        let recorded = mock2.recorded.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "20240102120000");
        assert_eq!(recorded[0].1, "add_avatar");
    }
}
