use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementKey {
    pub kind: String,
    pub identity: String,
}

/// Priority order so DEFINE TABLE comes before DEFINE FIELD/INDEX/EVENT on the same table.
fn kind_priority(kind: &str) -> u8 {
    match kind {
        "NAMESPACE" => 0,
        "DATABASE" => 1,
        "FUNCTION" => 2,
        "ANALYZER" => 3,
        "PARAM" => 4,
        "ACCESS" => 5,
        "API" => 6,
        "BUCKET" => 7,
        "TABLE" => 8,
        "FIELD" => 9,
        "INDEX" => 10,
        "EVENT" => 11,
        _ => 12,
    }
}

impl PartialOrd for StatementKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StatementKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        kind_priority(&self.kind)
            .cmp(&kind_priority(&other.kind))
            .then_with(|| self.identity.cmp(&other.identity))
    }
}

pub struct SchemaDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<(String, String)>,
}

impl SchemaDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    /// Format the diff as a migration file body.
    pub fn to_migration_string(&self) -> String {
        let mut parts = Vec::new();

        if !self.removed.is_empty() {
            parts.push("-- Removed".to_string());
            for stmt in &self.removed {
                parts.push(stmt.clone());
            }
            parts.push(String::new());
        }

        if !self.modified.is_empty() {
            parts.push("-- Modified".to_string());
            for (_, new_stmt) in &self.modified {
                parts.push(new_stmt.clone());
            }
            parts.push(String::new());
        }

        if !self.added.is_empty() {
            parts.push("-- Added".to_string());
            for stmt in &self.added {
                parts.push(stmt.clone());
            }
            parts.push(String::new());
        }

        parts.join("\n")
    }

    /// Print the diff in git-style colored format.
    pub fn print_colored(&self) {
        const RED: &str = "\x1b[31m";
        const GREEN: &str = "\x1b[32m";
        const YELLOW: &str = "\x1b[33m";
        const RESET: &str = "\x1b[0m";

        if !self.removed.is_empty() {
            for stmt in &self.removed {
                println!("  {RED}- {stmt}{RESET}");
            }
            println!();
        }

        if !self.added.is_empty() {
            for stmt in &self.added {
                println!("  {GREEN}+ {stmt}{RESET}");
            }
            println!();
        }

        if !self.modified.is_empty() {
            for (old_stmt, new_stmt) in &self.modified {
                println!("  {YELLOW}~ {old_stmt}{RESET}");
                println!("  {YELLOW}~ {new_stmt}{RESET}");
            }
            println!();
        }
    }
}

/// Split SurrealQL text into individual statements, respecting braces and string literals.
pub fn split_statements(schema: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut brace_depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev_char = '\0';

    for ch in schema.chars() {
        match ch {
            '\'' if !in_double_quote && prev_char != '\\' => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if !in_single_quote && prev_char != '\\' => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            '{' if !in_single_quote && !in_double_quote => {
                brace_depth += 1;
                current.push(ch);
            }
            '}' if !in_single_quote && !in_double_quote => {
                brace_depth -= 1;
                current.push(ch);
            }
            ';' if !in_single_quote && !in_double_quote && brace_depth <= 0 => {
                current.push(';');
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() && trimmed != ";" {
                    statements.push(trimmed);
                }
                current.clear();
                brace_depth = 0;
            }
            _ => {
                current.push(ch);
            }
        }
        prev_char = ch;
    }

    // Handle trailing statement without semicolon
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        statements.push(trimmed);
    }

    statements
}

/// Extract a unique key from a DEFINE or REMOVE statement.
pub fn extract_statement_key(stmt: &str) -> Option<StatementKey> {
    // Strip leading comment lines to find the actual statement
    let trimmed = stmt
        .lines()
        .map(|l| l.trim())
        .skip_while(|l| l.starts_with("--") || l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = trimmed.trim();

    if trimmed.is_empty() {
        return None;
    }

    // Normalize: remove OVERWRITE / IF NOT EXISTS for key extraction
    let normalized = trimmed
        .replace(" OVERWRITE ", " ")
        .replace(" IF NOT EXISTS ", " ");

    let tokens: Vec<&str> = normalized.split_whitespace().collect();

    if tokens.len() < 3 {
        return None;
    }

    // Only handle DEFINE statements
    if tokens[0] != "DEFINE" {
        return None;
    }

    let kind = tokens[1].to_uppercase();

    match kind.as_str() {
        "TABLE" => {
            let name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "TABLE".to_string(),
                identity: name,
            })
        }
        "FIELD" => {
            // DEFINE FIELD name ON [TABLE] table_name
            let field_name = tokens[2].to_lowercase();
            let table_name = find_on_table(&tokens)?;
            Some(StatementKey {
                kind: "FIELD".to_string(),
                identity: format!("{}/{}", table_name, field_name),
            })
        }
        "INDEX" => {
            // DEFINE INDEX name ON [TABLE] table_name
            let index_name = tokens[2].to_lowercase();
            let table_name = find_on_table(&tokens)?;
            Some(StatementKey {
                kind: "INDEX".to_string(),
                identity: format!("{}/{}", table_name, index_name),
            })
        }
        "EVENT" => {
            // DEFINE EVENT name ON [TABLE] table_name
            let event_name = tokens[2].to_lowercase();
            let table_name = find_on_table(&tokens)?;
            Some(StatementKey {
                kind: "EVENT".to_string(),
                identity: format!("{}/{}", table_name, event_name),
            })
        }
        "ACCESS" => {
            // DEFINE ACCESS name ON DATABASE
            let access_name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "ACCESS".to_string(),
                identity: access_name,
            })
        }
        "FUNCTION" => {
            // DEFINE FUNCTION fn::name or DEFINE FUNCTION OVERWRITE fn::name
            // Find the token that starts with "fn::" and strip any trailing parens/args
            let fn_name = tokens
                .iter()
                .find(|t| t.starts_with("fn::"))
                .copied()
                .or_else(|| tokens.get(2).copied())
                .unwrap_or("unknown");
            // Strip everything from the first '(' onward: "fn::register($name:" → "fn::register"
            let fn_name = fn_name.split('(').next().unwrap_or(fn_name);
            Some(StatementKey {
                kind: "FUNCTION".to_string(),
                identity: fn_name.to_lowercase(),
            })
        }
        "ANALYZER" => {
            let name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "ANALYZER".to_string(),
                identity: name,
            })
        }
        "PARAM" => {
            let name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "PARAM".to_string(),
                identity: name,
            })
        }
        "BUCKET" => {
            let name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "BUCKET".to_string(),
                identity: name,
            })
        }
        "API" => {
            // DEFINE API "/path" ...
            let name = tokens[2].to_lowercase();
            Some(StatementKey {
                kind: "API".to_string(),
                identity: name,
            })
        }
        _ => None,
    }
}

/// Find the table name after ON [TABLE] in a token list.
fn find_on_table(tokens: &[&str]) -> Option<String> {
    for i in 0..tokens.len() {
        if tokens[i].eq_ignore_ascii_case("ON") {
            // Next might be "TABLE" or the table name directly
            if i + 1 < tokens.len() {
                if tokens[i + 1].eq_ignore_ascii_case("TABLE") {
                    if i + 2 < tokens.len() {
                        return Some(tokens[i + 2].to_lowercase());
                    }
                } else {
                    return Some(tokens[i + 1].to_lowercase());
                }
            }
        }
    }
    None
}

/// Parse schema text into a key → statement map.
pub fn parse_schema_map(schema: &str) -> BTreeMap<StatementKey, String> {
    let statements = split_statements(schema);
    let mut map = BTreeMap::new();

    for stmt in statements {
        if let Some(key) = extract_statement_key(&stmt) {
            map.insert(key, stmt);
        }
    }

    map
}

/// Normalize a statement for comparison: collapse whitespace, trim.
fn normalize_for_compare(stmt: &str) -> String {
    stmt.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Diff two schemas. Returns SchemaDiff with additions, removals, modifications.
pub fn diff_schemas(old_schema: &str, new_schema: &str) -> SchemaDiff {
    let old_map = parse_schema_map(old_schema);
    let new_map = parse_schema_map(new_schema);

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    // Find additions and modifications
    for (key, new_stmt) in &new_map {
        match old_map.get(key) {
            None => added.push(new_stmt.clone()),
            Some(old_stmt) => {
                if normalize_for_compare(old_stmt) != normalize_for_compare(new_stmt) {
                    modified.push((old_stmt.clone(), new_stmt.clone()));
                }
            }
        }
    }

    // Find removals
    for (key, _) in &old_map {
        if !new_map.contains_key(key) {
            removed.push(generate_remove_statement(key));
        }
    }

    // Post-processing: detect potential field renames
    // A rename looks like a FIELD removal + FIELD addition on the same table with the same TYPE.
    let removed = detect_potential_renames(removed, &added, &old_map);

    SchemaDiff {
        added,
        removed,
        modified,
    }
}

/// Extract the TYPE clause from a DEFINE FIELD statement, if present.
fn extract_field_type(stmt: &str) -> Option<String> {
    let upper = stmt.to_uppercase();
    let type_pos = upper.find(" TYPE ")?;
    let after_type = &stmt[type_pos + 6..];
    // TYPE clause ends at the next known keyword or end of statement
    let end_keywords = [" DEFAULT ", " VALUE ", " ASSERT ", " PERMISSIONS ", " COMMENT ", " READONLY"];
    let end_pos = end_keywords
        .iter()
        .filter_map(|kw| after_type.to_uppercase().find(kw))
        .min()
        .unwrap_or(after_type.len());
    let type_str = after_type[..end_pos].trim().trim_end_matches(';').trim();
    if type_str.is_empty() {
        None
    } else {
        Some(type_str.to_lowercase())
    }
}

/// Check removed FIELD keys against added FIELD statements for potential renames.
/// Returns the removed list with rename candidates commented out as warnings.
fn detect_potential_renames(
    removed: Vec<String>,
    added: &[String],
    old_map: &BTreeMap<StatementKey, String>,
) -> Vec<String> {
    // Build a lookup: for each table, collect added fields with their types
    let mut added_fields_by_table: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for stmt in added {
        if let Some(key) = extract_statement_key(stmt) {
            if key.kind == "FIELD" {
                if let Some((table, field)) = key.identity.split_once('/') {
                    if let Some(field_type) = extract_field_type(stmt) {
                        added_fields_by_table
                            .entry(table.to_string())
                            .or_default()
                            .push((field.to_string(), field_type));
                    }
                }
            }
        }
    }

    let mut result = Vec::new();
    for remove_stmt in removed {
        // Try to parse as a FIELD removal
        if remove_stmt.contains("REMOVE FIELD") {
            // Extract the field name and table from the remove statement
            // Format: "REMOVE FIELD IF EXISTS {field} ON TABLE {table};"
            let is_rename_candidate = (|| {
                let upper = remove_stmt.to_uppercase();
                let field_pos = upper.find("REMOVE FIELD IF EXISTS ")
                    .map(|p| p + "REMOVE FIELD IF EXISTS ".len())
                    .or_else(|| upper.find("REMOVE FIELD ").map(|p| p + "REMOVE FIELD ".len()))?;
                let rest = &remove_stmt[field_pos..];
                let on_pos = rest.to_uppercase().find(" ON TABLE ")?;
                let field_name = rest[..on_pos].trim().to_lowercase();
                let table_name = rest[on_pos + " ON TABLE ".len()..].trim().trim_end_matches(';').trim().to_lowercase();

                // Look up the original DEFINE FIELD statement to get its type
                let old_key = StatementKey {
                    kind: "FIELD".to_string(),
                    identity: format!("{}/{}", table_name, field_name),
                };
                let old_stmt = old_map.get(&old_key)?;
                let old_type = extract_field_type(old_stmt)?;

                // Check if any added field on the same table has the same type
                let added_fields = added_fields_by_table.get(&table_name)?;
                let matching_add = added_fields.iter().find(|(_, t)| *t == old_type)?;

                Some((field_name, matching_add.0.clone(), table_name))
            })();

            if let Some((old_field, new_field, table)) = is_rename_candidate {
                result.push(format!(
                    "-- WARNING: Possible rename detected: {} -> {} on table {}\n\
                     -- If this was a rename, the REMOVE below is unnecessary (the prior migration handled it).\n\
                     -- Uncomment only if you are intentionally deleting this field.\n\
                     -- {};",
                    old_field, new_field, table, remove_stmt
                ));
            } else {
                result.push(remove_stmt);
            }
        } else {
            result.push(remove_stmt);
        }
    }

    result
}

/// Convert a StatementKey into the corresponding REMOVE statement.
fn generate_remove_statement(key: &StatementKey) -> String {
    match key.kind.as_str() {
        "TABLE" => format!("REMOVE TABLE IF EXISTS {};", key.identity),
        "FIELD" => {
            // identity is "table/field"
            if let Some((table, field)) = key.identity.split_once('/') {
                format!("REMOVE FIELD IF EXISTS {} ON TABLE {};", field, table)
            } else {
                format!("-- REMOVE FIELD {} (malformed key);", key.identity)
            }
        }
        "INDEX" => {
            if let Some((table, index)) = key.identity.split_once('/') {
                format!("REMOVE INDEX IF EXISTS {} ON TABLE {};", index, table)
            } else {
                format!("-- REMOVE INDEX {} (malformed key);", key.identity)
            }
        }
        "EVENT" => {
            if let Some((table, event)) = key.identity.split_once('/') {
                format!("REMOVE EVENT IF EXISTS {} ON TABLE {};", event, table)
            } else {
                format!("-- REMOVE EVENT {} (malformed key);", key.identity)
            }
        }
        "ACCESS" => format!("REMOVE ACCESS IF EXISTS {} ON DATABASE;", key.identity),
        "FUNCTION" => format!("REMOVE FUNCTION IF EXISTS {};", key.identity),
        "ANALYZER" => format!("REMOVE ANALYZER IF EXISTS {};", key.identity),
        "PARAM" => format!("REMOVE PARAM IF EXISTS {};", key.identity),
        "BUCKET" => format!("REMOVE BUCKET IF EXISTS {};", key.identity),
        "API" => format!("REMOVE API IF EXISTS {};", key.identity),
        _ => format!("-- REMOVE {} {};", key.kind, key.identity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── split_statements ─────────────────────────────────────────────

    #[test]
    fn test_split_simple_statements() {
        let input = "DEFINE TABLE user; DEFINE TABLE post;";
        let stmts = split_statements(input);
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "DEFINE TABLE user;");
        assert_eq!(stmts[1], "DEFINE TABLE post;");
    }

    #[test]
    fn test_split_with_braces() {
        let input = r#"DEFINE EVENT test ON TABLE user WHEN $event = "CREATE" THEN { LET $x = 1; LET $y = 2; };"#;
        let stmts = split_statements(input);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("LET $x = 1;"));
    }

    #[test]
    fn test_split_with_string_containing_semicolon() {
        let input = r#"DEFINE FIELD bio ON TABLE user TYPE string DEFAULT "hello; world";"#;
        let stmts = split_statements(input);
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_split_multiline() {
        let input = "DEFINE TABLE user SCHEMAFULL\n    PERMISSIONS FOR select WHERE true;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let stmts = split_statements(input);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn test_split_empty_input() {
        let stmts = split_statements("");
        assert!(stmts.is_empty());
    }

    #[test]
    fn test_split_comments_only() {
        let stmts = split_statements("-- just a comment\n-- another");
        assert_eq!(stmts.len(), 1); // The comment text is returned as a statement
    }

    // ── extract_statement_key ────────────────────────────────────────

    #[test]
    fn test_key_table() {
        let key = extract_statement_key("DEFINE TABLE user SCHEMAFULL;").unwrap();
        assert_eq!(key.kind, "TABLE");
        assert_eq!(key.identity, "user");
    }

    #[test]
    fn test_key_table_if_not_exists() {
        let key = extract_statement_key("DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;").unwrap();
        assert_eq!(key.kind, "TABLE");
        assert_eq!(key.identity, "user");
    }

    #[test]
    fn test_key_table_overwrite() {
        let key = extract_statement_key("DEFINE TABLE OVERWRITE user SCHEMAFULL;").unwrap();
        assert_eq!(key.kind, "TABLE");
        assert_eq!(key.identity, "user");
    }

    #[test]
    fn test_key_field() {
        let key = extract_statement_key("DEFINE FIELD name ON TABLE user TYPE string;").unwrap();
        assert_eq!(key.kind, "FIELD");
        assert_eq!(key.identity, "user/name");
    }

    #[test]
    fn test_key_field_without_table_keyword() {
        let key = extract_statement_key("DEFINE FIELD name ON user TYPE string;").unwrap();
        assert_eq!(key.kind, "FIELD");
        assert_eq!(key.identity, "user/name");
    }

    #[test]
    fn test_key_nested_field() {
        let key =
            extract_statement_key("DEFINE FIELD address.city ON TABLE user TYPE string;").unwrap();
        assert_eq!(key.kind, "FIELD");
        assert_eq!(key.identity, "user/address.city");
    }

    #[test]
    fn test_key_index() {
        let key = extract_statement_key(
            "DEFINE INDEX idx_email ON TABLE user FIELDS email UNIQUE;",
        )
        .unwrap();
        assert_eq!(key.kind, "INDEX");
        assert_eq!(key.identity, "user/idx_email");
    }

    #[test]
    fn test_key_event() {
        let key = extract_statement_key(
            r#"DEFINE EVENT on_create ON TABLE user WHEN $event = "CREATE" THEN { };"#,
        )
        .unwrap();
        assert_eq!(key.kind, "EVENT");
        assert_eq!(key.identity, "user/on_create");
    }

    #[test]
    fn test_key_function() {
        let key =
            extract_statement_key("DEFINE FUNCTION fn::register($name: string) { RETURN true; };")
                .unwrap();
        assert_eq!(key.kind, "FUNCTION");
        assert_eq!(key.identity, "fn::register");
    }

    #[test]
    fn test_key_access() {
        let key = extract_statement_key(
            "DEFINE ACCESS user_access ON DATABASE TYPE RECORD SIGNIN (...);",
        )
        .unwrap();
        assert_eq!(key.kind, "ACCESS");
        assert_eq!(key.identity, "user_access");
    }

    #[test]
    fn test_key_comment_returns_none() {
        assert!(extract_statement_key("-- this is a comment").is_none());
    }

    #[test]
    fn test_key_empty_returns_none() {
        assert!(extract_statement_key("").is_none());
    }

    // ── diff_schemas ─────────────────────────────────────────────────

    #[test]
    fn test_diff_no_changes() {
        let schema = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let diff = diff_schemas(schema, schema);
        assert!(diff.is_empty());
    }

    #[test]
    fn test_diff_addition() {
        let old = "DEFINE TABLE user SCHEMAFULL;";
        let new = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let diff = diff_schemas(old, new);
        assert_eq!(diff.added.len(), 1);
        assert!(diff.added[0].contains("DEFINE FIELD name"));
        assert!(diff.removed.is_empty());
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn test_diff_removal() {
        let old = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let new = "DEFINE TABLE user SCHEMAFULL;";
        let diff = diff_schemas(old, new);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert!(diff.removed[0].contains("REMOVE FIELD IF EXISTS name ON TABLE user"));
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn test_diff_modification() {
        let old = "DEFINE FIELD name ON TABLE user TYPE string;";
        let new = "DEFINE FIELD name ON TABLE user TYPE option<string>;";
        let diff = diff_schemas(old, new);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.modified.len(), 1);
        assert!(diff.modified[0].0.contains("TYPE string"));
        assert!(diff.modified[0].1.contains("option<string>"));
    }

    #[test]
    fn test_diff_mixed_changes() {
        let old = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;\nDEFINE FIELD old_field ON TABLE user TYPE string;";
        let new = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE option<string>;\nDEFINE FIELD avatar ON TABLE user TYPE string;";
        let diff = diff_schemas(old, new);
        assert_eq!(diff.added.len(), 1); // avatar
        assert_eq!(diff.removed.len(), 1); // old_field
        assert_eq!(diff.modified.len(), 1); // name type changed
    }

    #[test]
    fn test_diff_empty_old_schema() {
        let new = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let diff = diff_schemas("", new);
        assert_eq!(diff.added.len(), 2);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn test_diff_empty_new_schema() {
        let old = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let diff = diff_schemas(old, "");
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 2);
    }

    // ── generate_remove_statement ────────────────────────────────────

    #[test]
    fn test_remove_table() {
        let key = StatementKey {
            kind: "TABLE".to_string(),
            identity: "user".to_string(),
        };
        assert_eq!(generate_remove_statement(&key), "REMOVE TABLE IF EXISTS user;");
    }

    #[test]
    fn test_remove_field() {
        let key = StatementKey {
            kind: "FIELD".to_string(),
            identity: "user/name".to_string(),
        };
        assert_eq!(
            generate_remove_statement(&key),
            "REMOVE FIELD IF EXISTS name ON TABLE user;"
        );
    }

    #[test]
    fn test_remove_index() {
        let key = StatementKey {
            kind: "INDEX".to_string(),
            identity: "user/idx_email".to_string(),
        };
        assert_eq!(
            generate_remove_statement(&key),
            "REMOVE INDEX IF EXISTS idx_email ON TABLE user;"
        );
    }

    #[test]
    fn test_remove_function() {
        let key = StatementKey {
            kind: "FUNCTION".to_string(),
            identity: "fn::register".to_string(),
        };
        assert_eq!(
            generate_remove_statement(&key),
            "REMOVE FUNCTION IF EXISTS fn::register;"
        );
    }

    // ── to_migration_string ──────────────────────────────────────────

    #[test]
    fn test_migration_string_format() {
        let diff = SchemaDiff {
            removed: vec!["REMOVE FIELD IF EXISTS old ON TABLE user;".to_string()],
            modified: vec![("DEFINE TABLE user SCHEMALESS;".to_string(), "DEFINE TABLE user SCHEMAFULL;".to_string())],
            added: vec!["DEFINE FIELD avatar ON TABLE user TYPE string;".to_string()],
        };
        let output = diff.to_migration_string();
        assert!(output.contains("-- Removed"));
        assert!(output.contains("-- Modified"));
        assert!(output.contains("-- Added"));
        // Removals come first
        let removed_pos = output.find("-- Removed").unwrap();
        let modified_pos = output.find("-- Modified").unwrap();
        let added_pos = output.find("-- Added").unwrap();
        assert!(removed_pos < modified_pos);
        assert!(modified_pos < added_pos);
    }

    #[test]
    fn test_migration_string_empty_diff() {
        let diff = SchemaDiff {
            removed: vec![],
            modified: vec![],
            added: vec![],
        };
        let output = diff.to_migration_string();
        assert!(output.trim().is_empty());
    }

    // ── parse_schema_map ─────────────────────────────────────────────

    #[test]
    fn test_parse_schema_map() {
        let schema = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;\nDEFINE INDEX idx_email ON TABLE user FIELDS email UNIQUE;";
        let map = parse_schema_map(schema);
        assert_eq!(map.len(), 3);
        assert!(map.contains_key(&StatementKey {
            kind: "TABLE".to_string(),
            identity: "user".to_string(),
        }));
        assert!(map.contains_key(&StatementKey {
            kind: "FIELD".to_string(),
            identity: "user/name".to_string(),
        }));
    }

    // ── detect_potential_renames ────────────────────────────────────

    #[test]
    fn test_diff_detects_potential_rename() {
        let old = "DEFINE TABLE conversation SCHEMAFULL;\nDEFINE FIELD session ON TABLE conversation TYPE string;";
        let new = "DEFINE TABLE conversation SCHEMAFULL;\nDEFINE FIELD chat_session ON TABLE conversation TYPE string;";
        let diff = diff_schemas(old, new);
        // The removal should be commented out as a rename warning
        assert_eq!(diff.removed.len(), 1);
        assert!(diff.removed[0].contains("WARNING: Possible rename detected"));
        assert!(diff.removed[0].contains("session -> chat_session"));
        assert!(diff.removed[0].contains("on table conversation"));
        // The actual REMOVE should be commented out
        assert!(diff.removed[0].contains("-- REMOVE FIELD IF EXISTS session ON TABLE conversation;"));
    }

    #[test]
    fn test_diff_no_rename_different_types() {
        let old = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD age ON TABLE user TYPE int;";
        let new = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;";
        let diff = diff_schemas(old, new);
        // Different types → not a rename candidate, REMOVE should be plain
        assert_eq!(diff.removed.len(), 1);
        assert!(!diff.removed[0].contains("WARNING"));
        assert!(diff.removed[0].contains("REMOVE FIELD IF EXISTS age ON TABLE user;"));
    }

    #[test]
    fn test_diff_no_rename_different_tables() {
        let old = "DEFINE TABLE user SCHEMAFULL;\nDEFINE FIELD name ON TABLE user TYPE string;\nDEFINE TABLE post SCHEMAFULL;";
        let new = "DEFINE TABLE user SCHEMAFULL;\nDEFINE TABLE post SCHEMAFULL;\nDEFINE FIELD title ON TABLE post TYPE string;";
        let diff = diff_schemas(old, new);
        // Removal on user, addition on post → not a rename
        assert_eq!(diff.removed.len(), 1);
        assert!(!diff.removed[0].contains("WARNING"));
    }

    // ── extract_field_type ──────────────────────────────────────────

    #[test]
    fn test_extract_field_type_simple() {
        let stmt = "DEFINE FIELD name ON TABLE user TYPE string;";
        assert_eq!(extract_field_type(stmt).unwrap(), "string");
    }

    #[test]
    fn test_extract_field_type_with_default() {
        let stmt = "DEFINE FIELD status ON TABLE user TYPE string DEFAULT 'active';";
        assert_eq!(extract_field_type(stmt).unwrap(), "string");
    }

    #[test]
    fn test_extract_field_type_option() {
        let stmt = "DEFINE FIELD bio ON TABLE user TYPE option<string>;";
        assert_eq!(extract_field_type(stmt).unwrap(), "option<string>");
    }

    #[test]
    fn test_extract_field_type_no_type() {
        let stmt = "DEFINE FIELD computed ON TABLE user VALUE $this.a + $this.b;";
        assert!(extract_field_type(stmt).is_none());
    }

    #[test]
    fn test_overwrite_vs_if_not_exists_same_key() {
        let old = "DEFINE TABLE OVERWRITE user SCHEMAFULL;";
        let new = "DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;";
        let old_map = parse_schema_map(old);
        let new_map = parse_schema_map(new);

        // Same key
        let key = StatementKey {
            kind: "TABLE".to_string(),
            identity: "user".to_string(),
        };
        assert!(old_map.contains_key(&key));
        assert!(new_map.contains_key(&key));
    }
}
