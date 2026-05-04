use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use crate::annotations;
use crate::backend::{BackendProcessor, DeployMode};
use crate::parser::SchemaParser;
use regex::Regex;

pub struct SchemaBuilderConfig {
    pub input_path: PathBuf,
    pub config_path: Option<PathBuf>,
    pub mode: DeployMode,
    pub endpoint: Option<String>,
    pub secret: Option<String>,
    pub include_functions: bool,
}

/// Build ONLY the remote functions SQL (heartbeat + mode-specific functions
/// with endpoint/secret substitution).  Used by `dev.rs` to apply functions
/// separately with Docker-internal URLs.
pub fn build_remote_functions_schema(
    mode: &DeployMode,
    endpoint: &str,
    secret: &str,
) -> String {
    let mut content = String::new();

    // Set database-level params so events can reference them without hardcoding
    content.push_str(&format!(
        "DEFINE PARAM OVERWRITE $sp00ky_endpoint VALUE '{}';\n",
        endpoint
    ));
    content.push_str(&format!(
        "DEFINE PARAM OVERWRITE $sp00ky_secret VALUE '{}';\n\n",
        secret
    ));

    // Common remote functions (heartbeat)
    content.push_str(include_str!("functions_remote.surql"));

    // Mode-specific functions
    let functions_remote_singlenode = include_str!("functions_remote_singlenode.surql");
    let functions_remote_surrealism = include_str!("functions_remote_surrealism.surql");

    if *mode == DeployMode::Singlenode || *mode == DeployMode::Cluster {
        let mut singlenode_fn = functions_remote_singlenode.to_string();
        singlenode_fn = singlenode_fn.replace("{{ENDPOINT}}", endpoint);
        singlenode_fn = singlenode_fn.replace("{{SECRET}}", secret);

        content.push('\n');
        content.push_str(&singlenode_fn);
    } else if *mode == DeployMode::Surrealism {
        content.push('\n');
        content.push_str(functions_remote_surrealism);
    }

    content
}

/// Assembles the complete server schema from all sources.
///
/// This builds the full schema that should be present in SurrealDB:
/// user schema + backend schemas + meta tables + remote functions + buckets.
pub fn build_server_schema(config: &SchemaBuilderConfig) -> Result<String> {
    let mut content = fs::read_to_string(&config.input_path).context(format!(
        "Failed to read input schema file: {:?}",
        config.input_path
    ))?;

    // Process sp00ky config/backends
    let mut backend_processor = BackendProcessor::new();
    if let Some(config_path) = &config.config_path {
        if config_path.exists() {
            backend_processor.process(config_path)?;
            content.push('\n');
            content.push_str(&backend_processor.schema_appends);
        }
    }

    // Base meta tables
    content.push('\n');
    content.push_str(include_str!("meta_tables.surql"));

    // Remote meta tables (server-side)
    content.push('\n');
    content.push_str(include_str!("meta_tables_remote.surql"));

    // Migration tracking table
    content.push('\n');
    content.push_str(include_str!("migration_tables.surql"));

    // Bucket definitions
    if !backend_processor.bucket_schema.is_empty() {
        content.push('\n');
        content.push_str(&backend_processor.bucket_schema);
    }

    // Remote functions — only when include_functions is true
    if config.include_functions {
        let default_endpoint = if config.mode == DeployMode::Cluster {
            "http://localhost:9667"
        } else {
            "http://localhost:8667"
        };
        let endpoint = config
            .endpoint
            .as_deref()
            .unwrap_or(default_endpoint);
        let secret = config.secret.as_deref().unwrap_or("");

        let functions_sql = build_remote_functions_schema(&config.mode, endpoint, secret);
        content.push('\n');
        content.push_str(&functions_sql);
    }

    // Substitute the {{CRDT_UPDATE_RULE}} placeholder in
    // meta_tables_remote.surql with a derived rule that ORs each
    // CRDT-bearing parent's UPDATE expression rewritten to dereference
    // through `record_id.<field>`. SurrealDB permission expressions run
    // in an elevated context, so inline UPDATE checks on the parent
    // table don't actually enforce its rule — only field dereference
    // does (it runs the parent's SELECT rule on the way through, and
    // a comparison like `record_id.author.id = $auth.id` still gates
    // on identity).
    let crdt_rule = build_crdt_update_rule(&config.input_path)?;
    content = content.replace("{{CRDT_UPDATE_RULE}}", &crdt_rule);

    // Replace unregister_view call (this transforms event handlers in user
    // schema, not function definitions — always apply it)
    if config.mode == DeployMode::Singlenode || config.mode == DeployMode::Cluster {
        let unregister_call = "let $result = mod::dbsp::unregister_view(<string>$before.id);";
        let unregister_http =
            "let $payload = { id: <string>$before.id };\n    let $result = http::post($sp00ky_endpoint + '/view/unregister', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });";
        content = content.replace(unregister_call, unregister_http);
    }

    Ok(content)
}

/// Build the `FOR create, update WHERE ...` expression for `_00_crdt` and
/// `_00_cursor`. For each parent table that has at least one `@crdt`-annotated
/// field, take its table-level UPDATE expression and rewrite all
/// table-local field references to dereference through `record_id.`.
/// Then OR all per-parent expressions together, each guarded by
/// `record_id.tb() = "<parent>"`.
///
/// If no CRDT-bearing parent exists the rule collapses to `false` (no row
/// can be written, which is fine — the meta tables are unused).
fn build_crdt_update_rule(input_path: &std::path::Path) -> Result<String> {
    let source = fs::read_to_string(input_path).context(format!(
        "Failed to read input schema for CRDT permission derivation: {:?}",
        input_path
    ))?;
    let mut parser = SchemaParser::new();
    parser
        .parse_file(&source)
        .context("Failed to parse user schema for CRDT permission derivation")?;
    Ok(build_crdt_update_rule_from(&source, &parser))
}

/// Same as `build_crdt_update_rule` but accepts an already-parsed schema
/// and the original source. Used from places that have these in hand
/// (e.g. `migrate.rs`, `main.rs` codegen) so we don't re-parse.
pub fn build_crdt_update_rule_from(source: &str, parser: &SchemaParser) -> String {
    let field_annotations = annotations::extract_field_annotations(source);
    let mut crdt_parents: BTreeSet<String> = BTreeSet::new();
    for ((table, _), anns) in &field_annotations {
        if anns.iter().any(|a| a.name == "crdt") {
            crdt_parents.insert(table.clone());
        }
    }
    if crdt_parents.is_empty() {
        return "false".to_string();
    }

    let mut clauses: Vec<String> = Vec::new();
    for parent in &crdt_parents {
        let table = match parser.tables.get(parent) {
            Some(t) => t,
            None => continue,
        };
        let raw = table
            .table_permissions
            .get("update")
            .cloned()
            .unwrap_or_else(|| "false".to_string());
        let local_fields: BTreeSet<String> = table.fields.keys().cloned().collect();
        let rewritten = rewrite_idioms_for_meta(&raw, &local_fields);
        clauses.push(format!(
            "(record_id.tb() = \"{}\" AND ({}))",
            parent, rewritten
        ));
    }

    if clauses.is_empty() {
        "false".to_string()
    } else {
        clauses.join(" OR ")
    }
}

/// Substitute the `{{CRDT_UPDATE_RULE}}` placeholder in any string that
/// embeds `meta_tables_remote.surql`. Centralized here so every code path
/// that includes the meta tables (full-schema build in `schema_builder.rs`,
/// migration internal SQL in `migrate.rs`, codegen in `main.rs`) renders
/// the rule the same way.
pub fn substitute_crdt_update_rule(content: &str, source: &str, parser: &SchemaParser) -> String {
    let rule = build_crdt_update_rule_from(source, parser);
    content.replace("{{CRDT_UPDATE_RULE}}", &rule)
}

/// Prefix bare field references with `record_id.` and re-anchor `$parent`
/// at `record_id`. Used by `build_crdt_update_rule` to turn a parent
/// table's UPDATE expression into one that runs on a `_00_crdt` /
/// `_00_cursor` row referencing that parent.
///
/// Only tokens that exactly match a name in `local_fields` are rewritten,
/// and only when not already qualified by `.` or `$`. This is a regex-based
/// rewrite — sufficient for the simple WHERE expressions used in the example
/// schema; deeply nested subqueries with their own table-local fields may
/// need manual review.
pub fn rewrite_idioms_for_meta(expr: &str, local_fields: &BTreeSet<String>) -> String {
    let mut out = expr.to_string();

    // `$parent.id` — in nested subqueries this refers to the outer
    // enclosing record. Originally it points at the parent table's row;
    // when the rule is on a meta table the outer enclosing record is the
    // meta row, and we want its `record_id` field (which holds the parent
    // record id). So `$parent.id` → `$parent.record_id`. We keep the
    // `$parent.` prefix because subquery scoping resolves bare idioms
    // against the subquery row, not the outer scope.
    let parent_id_re = Regex::new(r"\$parent\.id\b").unwrap();
    out = parent_id_re.replace_all(&out, "$$parent.record_id").to_string();

    // Each table-local field name → `record_id.<field>`, but only when
    // it appears as a bare identifier (not preceded by `.` or `$`, not
    // already part of a longer word). The standard `regex` crate has no
    // lookbehind, so the boundary char is captured and re-emitted.
    for f in local_fields {
        let pat = format!(r"(^|[^\w.$]){}\b", regex::escape(f));
        let re = Regex::new(&pat).expect("static pattern");
        out = re
            .replace_all(&out, format!("${{1}}record_id.{}", f))
            .to_string();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fields(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rewrites_simple_field_dereference() {
        let f = fields(&["author", "title"]);
        let got = rewrite_idioms_for_meta("author.id = $auth.id", &f);
        assert_eq!(got, "record_id.author.id = $auth.id");
    }

    #[test]
    fn rewrites_field_after_open_paren() {
        let f = fields(&["author"]);
        let got = rewrite_idioms_for_meta("(author.id = $auth.id)", &f);
        assert_eq!(got, "(record_id.author.id = $auth.id)");
    }

    #[test]
    fn does_not_rewrite_qualified_field() {
        let f = fields(&["author"]);
        // Already qualified — leave alone.
        let got = rewrite_idioms_for_meta("$thing.author.id", &f);
        assert_eq!(got, "$thing.author.id");
    }

    #[test]
    fn rewrites_parent_id_to_parent_record_id() {
        let f = fields(&[]);
        let got = rewrite_idioms_for_meta("out = $parent.id", &f);
        // Keeps `$parent.` so subquery scoping still resolves it against
        // the outer (meta) row rather than the subquery's own record.
        assert_eq!(got, "out = $parent.record_id");
    }

    #[test]
    fn rewrites_thread_update_rule() {
        let f = fields(&["author", "title", "content", "active", "created_at"]);
        let raw = r#"$access = 'account' AND (author.id = $auth.id OR $auth.id INSIDE (SELECT VALUE in FROM collaborates_on WHERE out = $parent.id))"#;
        let got = rewrite_idioms_for_meta(raw, &f);
        assert!(
            got.contains("record_id.author.id = $auth.id"),
            "expected `record_id.author.id` in output: {got}"
        );
        assert!(
            got.contains("out = $parent.record_id"),
            "expected `out = $parent.record_id` in output: {got}"
        );
    }
}
