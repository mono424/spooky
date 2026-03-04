use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StatementKey {
    pub kind: String,
    pub identity: String,
}

pub struct SchemaDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
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
            for stmt in &self.modified {
                parts.push(stmt.clone());
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
                    modified.push(new_stmt.clone());
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

    SchemaDiff {
        added,
        removed,
        modified,
    }
}

/// Convert a StatementKey into the corresponding REMOVE statement.
fn generate_remove_statement(key: &StatementKey) -> String {
    match key.kind.as_str() {
        "TABLE" => format!("REMOVE TABLE {};", key.identity),
        "FIELD" => {
            // identity is "table/field"
            if let Some((table, field)) = key.identity.split_once('/') {
                format!("REMOVE FIELD {} ON TABLE {};", field, table)
            } else {
                format!("-- REMOVE FIELD {} (malformed key);", key.identity)
            }
        }
        "INDEX" => {
            if let Some((table, index)) = key.identity.split_once('/') {
                format!("REMOVE INDEX {} ON TABLE {};", index, table)
            } else {
                format!("-- REMOVE INDEX {} (malformed key);", key.identity)
            }
        }
        "EVENT" => {
            if let Some((table, event)) = key.identity.split_once('/') {
                format!("REMOVE EVENT {} ON TABLE {};", event, table)
            } else {
                format!("-- REMOVE EVENT {} (malformed key);", key.identity)
            }
        }
        "ACCESS" => format!("REMOVE ACCESS {} ON DATABASE;", key.identity),
        "FUNCTION" => format!("REMOVE FUNCTION {};", key.identity),
        "ANALYZER" => format!("REMOVE ANALYZER {};", key.identity),
        "PARAM" => format!("REMOVE PARAM {};", key.identity),
        "BUCKET" => format!("REMOVE BUCKET {};", key.identity),
        "API" => format!("REMOVE API {};", key.identity),
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
        assert!(diff.removed[0].contains("REMOVE FIELD name ON TABLE user"));
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
        assert!(diff.modified[0].contains("option<string>"));
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
        assert_eq!(generate_remove_statement(&key), "REMOVE TABLE user;");
    }

    #[test]
    fn test_remove_field() {
        let key = StatementKey {
            kind: "FIELD".to_string(),
            identity: "user/name".to_string(),
        };
        assert_eq!(
            generate_remove_statement(&key),
            "REMOVE FIELD name ON TABLE user;"
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
            "REMOVE INDEX idx_email ON TABLE user;"
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
            "REMOVE FUNCTION fn::register;"
        );
    }

    // ── to_migration_string ──────────────────────────────────────────

    #[test]
    fn test_migration_string_format() {
        let diff = SchemaDiff {
            removed: vec!["REMOVE FIELD old ON TABLE user;".to_string()],
            modified: vec!["DEFINE TABLE user SCHEMAFULL;".to_string()],
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
