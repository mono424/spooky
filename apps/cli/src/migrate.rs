use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use crate::surreal_client::MigrationDB;

/// A migration discovered on the filesystem.
pub(crate) struct FilesystemMigration {
    pub version: String,
    pub name: String,
    pub path: PathBuf,
}

/// Scan the migrations directory and return sorted migrations.
///
/// Expects flat `.surql` files named `{14-digit-timestamp}_{name}.surql`.
pub(crate) fn scan_migrations(migrations_dir: &Path) -> Result<Vec<FilesystemMigration>> {
    if !migrations_dir.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    for entry in fs::read_dir(migrations_dir).context("Failed to read migrations directory")? {
        let entry = entry?;
        let path = entry.path();

        // Only consider .surql files
        if path.is_dir() || path.extension().and_then(|e| e.to_str()) != Some("surql") {
            continue;
        }

        let file_stem = match path.file_stem().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Parse "{version}_{name}" pattern
        let underscore_pos = match file_stem.find('_') {
            Some(pos) => pos,
            None => continue,
        };

        let version = file_stem[..underscore_pos].to_string();
        let name = file_stem[underscore_pos + 1..].to_string();

        // Validate version looks like a timestamp
        if version.len() != 14 || !version.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        migrations.push(FilesystemMigration {
            version,
            name,
            path,
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

/// Create a new migration file.
pub fn create(migrations_dir: &Path, name: &str, schema_path: Option<&Path>) -> Result<()> {
    let sanitized = sanitize_name(name);
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let file_name = format!("{}_{}.surql", timestamp, sanitized);
    let file_path = migrations_dir.join(&file_name);

    fs::create_dir_all(migrations_dir)
        .context(format!("Failed to create migrations directory: {:?}", migrations_dir))?;

    let content = if let Some(schema) = schema_path {
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

    fs::write(&file_path, content)
        .context(format!("Failed to write migration file: {:?}", file_path))?;

    println!("Created migration: {}", file_name);
    println!("  {}", file_path.display());

    Ok(())
}

/// Apply all pending migrations in order.
pub fn apply(client: &dyn MigrationDB, migrations_dir: &Path) -> Result<()> {
    client.ensure_ns_db().context("Failed to ensure namespace/database exist")?;
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let applied = client.get_applied_migrations()?;
    let filesystem = scan_migrations(migrations_dir)?;

    // Integrity check: verify checksums of applied migrations
    for am in &applied {
        if let Some(fm) = filesystem.iter().find(|f| f.version == am.version) {
            if !fm.path.exists() {
                println!(
                    "WARNING: Applied migration {}_{} is missing from disk",
                    am.version, am.name
                );
                continue;
            }
            let disk_checksum = checksum(&fm.path)?;
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
        if !migration.path.exists() {
            bail!(
                "Missing migration file for {}_{}",
                migration.version,
                migration.name
            );
        }

        let sql = fs::read_to_string(&migration.path)
            .context(format!("Failed to read {:?}", migration.path))?;

        let hash = checksum(&migration.path)?;

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
            if fm.path.exists() {
                let disk_checksum = checksum(&fm.path)?;
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
        fail_execute: RefCell<bool>,
    }

    impl MockDB {
        fn new() -> Self {
            Self {
                applied: RefCell::new(vec![]),
                executed_queries: RefCell::new(vec![]),
                recorded: RefCell::new(vec![]),
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

        fn ensure_ns_db(&self) -> Result<()> {
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
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Create a flat migration file in the migrations directory.
    fn create_migration_file(
        base: &Path,
        version: &str,
        name: &str,
        content: &str,
    ) -> PathBuf {
        fs::create_dir_all(base).unwrap();
        let file_path = base.join(format!("{}_{}.surql", version, name));
        fs::write(&file_path, content).unwrap();
        file_path
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
        create_migration_file(dir.path(), "20240101120000", "initial", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "20240101120000");
        assert_eq!(result[0].name, "initial");
    }

    #[test]
    fn test_scan_sorts_by_version() {
        let dir = TempDir::new().unwrap();
        create_migration_file(dir.path(), "20240103120000", "third", "-- up");
        create_migration_file(dir.path(), "20240101120000", "first", "-- up");
        create_migration_file(dir.path(), "20240102120000", "second", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].version, "20240101120000");
        assert_eq!(result[1].version, "20240102120000");
        assert_eq!(result[2].version, "20240103120000");
    }

    #[test]
    fn test_scan_skips_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("20240101120000_not_a_file")).unwrap();
        create_migration_file(dir.path(), "20240101120000", "real", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "real");
    }

    #[test]
    fn test_scan_skips_non_surql_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("20240101120000_readme.txt"), "text").unwrap();
        create_migration_file(dir.path(), "20240101120000", "real", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "real");
    }

    #[test]
    fn test_scan_skips_files_without_underscore() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("nounderscore.surql"), "-- sql").unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_skips_files_with_short_version() {
        let dir = TempDir::new().unwrap();
        // Version must be exactly 14 digits
        fs::write(dir.path().join("2024_too_short.surql"), "-- sql").unwrap();
        fs::write(dir.path().join("12345_short.surql"), "-- sql").unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_skips_files_with_non_digit_version() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("2024010112000a_bad_ver.surql"), "-- sql").unwrap();

        let result = scan_migrations(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_sets_correct_path() {
        let dir = TempDir::new().unwrap();
        create_migration_file(dir.path(), "20240101120000", "test", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result[0].path, dir.path().join("20240101120000_test.surql"));
    }

    #[test]
    fn test_scan_handles_multi_underscore_names() {
        let dir = TempDir::new().unwrap();
        create_migration_file(dir.path(), "20240101120000", "add_user_table", "-- up");

        let result = scan_migrations(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        // The name should be everything after the first underscore
        assert_eq!(result[0].name, "add_user_table");
    }

    // ── create tests ────────────────────────────────────────────────────

    #[test]
    fn test_create_makes_file() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "initial", None).unwrap();

        let entries: Vec<_> = fs::read_dir(&migrations_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);

        let path = entries[0].path();
        assert!(path.extension().unwrap() == "surql");
        assert!(path.is_file());
    }

    #[test]
    fn test_create_uses_timestamp_name_pattern() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "add_users", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let name = entry.file_name().to_string_lossy().to_string();

        // Format: {14-digit timestamp}_{sanitized_name}.surql
        assert!(name.ends_with("_add_users.surql"));
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
        assert!(name.ends_with("_add_user_avatar.surql"));
    }

    #[test]
    fn test_create_has_header_comment() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        create(&migrations_dir, "test", None).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let content = fs::read_to_string(entry.path()).unwrap();
        assert!(content.starts_with("-- Migration: test"));
        assert!(content.contains("-- Created:"));
        assert!(content.contains("Write your forward migration"));
    }

    #[test]
    fn test_create_with_schema_prepopulates() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        // Create a schema file
        let schema_path = dir.path().join("schema.surql");
        fs::write(&schema_path, "DEFINE TABLE user SCHEMAFULL;").unwrap();

        create(&migrations_dir, "initial", Some(&schema_path)).unwrap();

        let entry = fs::read_dir(&migrations_dir).unwrap().next().unwrap().unwrap();
        let content = fs::read_to_string(entry.path()).unwrap();
        assert!(content.contains("DEFINE TABLE user SCHEMAFULL;"));
        assert!(content.contains("WARNING"));
        assert!(content.contains("ONLY the incremental changes"));
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
        create_migration_file(dir.path(), "20240101120000", "initial", up_sql);

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
        create_migration_file(dir.path(), "20240101120000", "first", "CREATE first;");
        create_migration_file(dir.path(), "20240102120000", "second", "CREATE second;");
        create_migration_file(dir.path(), "20240103120000", "third", "CREATE third;");

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
        create_migration_file(dir.path(), "20240101120000", "first", up_sql);
        create_migration_file(dir.path(), "20240102120000", "second", "CREATE second;");

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
        create_migration_file(dir.path(), "20240101120000", "initial", "MODIFIED content");

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
    fn test_apply_execute_failure_aborts() {
        let dir = TempDir::new().unwrap();
        create_migration_file(dir.path(), "20240101120000", "initial", "INVALID SQL;");

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
        create_migration_file(dir.path(), "20240101120000", "initial", up_sql);

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "initial",
            &checksum_str(up_sql),
        )]);

        apply(&mock, dir.path()).unwrap();
        assert!(mock.recorded.borrow().is_empty());
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
        create_migration_file(dir.path(), "20240101120000", "pending", "-- up");

        let mock = MockDB::new();
        // Should succeed without error
        status(&mock, dir.path()).unwrap();
    }

    #[test]
    fn test_status_with_applied_and_pending() {
        let dir = TempDir::new().unwrap();
        let up_sql = "CREATE first;";
        create_migration_file(dir.path(), "20240101120000", "applied", up_sql);
        create_migration_file(dir.path(), "20240102120000", "pending", "CREATE second;");

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
        create_migration_file(dir.path(), "20240101120000", "drifted", "MODIFIED content");

        let mock = MockDB::with_applied(vec![make_applied(
            "20240101120000",
            "drifted",
            "original_checksum_doesnt_match",
        )]);

        // Should succeed (drift is a warning, not an error in status)
        status(&mock, dir.path()).unwrap();
    }

    // ── Integration-style tests (filesystem + mock DB) ──────────────────

    #[test]
    fn test_full_lifecycle_create_then_apply() {
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

        let mig_path = entries[0].path();
        let up_sql = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON user TYPE string;";
        fs::write(&mig_path, up_sql).unwrap();

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
    }

    #[test]
    fn test_apply_then_create_new_then_apply_again() {
        let dir = TempDir::new().unwrap();
        let migrations_dir = dir.path().join("migrations");

        // First migration
        let up1 = "DEFINE TABLE user SCHEMAFULL;";
        create_migration_file(&migrations_dir, "20240101120000", "initial", up1);

        // Apply first
        let mock = MockDB::new();
        apply(&mock, &migrations_dir).unwrap();
        assert_eq!(mock.recorded.borrow().len(), 1);

        // Add second migration
        let up2 = "DEFINE FIELD avatar ON user TYPE string;";
        create_migration_file(&migrations_dir, "20240102120000", "add_avatar", up2);

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
