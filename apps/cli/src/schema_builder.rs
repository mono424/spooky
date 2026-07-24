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
pub fn build_remote_functions_schema(mode: &DeployMode, endpoint: &str, secret: &str) -> String {
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

/// Platform-written job fields injected onto every outbox table.
///
/// The SSP stamps `assignee` (`UPDATE <job> SET assignee = "ssp-0"`) when it
/// picks up or recovers a job. Users define their outbox tables themselves
/// (usually SCHEMAFULL) and the documented template never mentioned this
/// platform-internal field, so the stamp was silently rejected — harmless on
/// the CREATE-pickup path (which tolerates it), fatal on the recovery path:
/// `/job/recover` aborts before enqueueing when it can't claim, so a stuck
/// job is re-dispatched by the sweep every 30s forever without ever running.
///
/// `IF NOT EXISTS` (not OVERWRITE) so an explicit user definition is never
/// clobbered. Emitted by both schema paths: `migrate::apply_internal_schema`
/// (VM) and `build_server_schema` (free/Cloudflare push).
pub fn build_outbox_platform_fields<'a, I>(outbox_tables: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = String::new();
    for table in outbox_tables {
        if table.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "DEFINE FIELD IF NOT EXISTS assignee ON {} TYPE option<string> \
             PERMISSIONS FOR select WHERE true FOR create, update WHERE false;\n",
            table
        ));
    }
    out
}

/// Collect the outbox table names a processed `BackendProcessor` discovered.
pub fn outbox_tables(processor: &BackendProcessor) -> Vec<String> {
    processor
        .backend_definitions
        .values()
        .filter_map(|def| def.outbox_table.clone())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Assembles the complete server schema from all sources.
///
/// This builds the full schema that should be present in SurrealDB:
/// user schema + backend schemas + meta tables + remote functions + buckets.
pub fn build_server_schema(config: &SchemaBuilderConfig) -> Result<String> {
    let raw_content = fs::read_to_string(&config.input_path).context(format!(
        "Failed to read input schema file: {:?}",
        config.input_path
    ))?;

    // Apply the `@crdt @cursor` TYPE rewrite to user DEFINE FIELDs before
    // any of the meta tables / functions are appended. Both client and
    // server emit the same shape for these fields.
    let mut content = rewrite_crdt_cursor_fields(&raw_content);

    // Bake the `@nosync` marker into the DEFINE TABLE of any table marked
    // `-- @nosync`, so the runtime services can detect it via `INFO FOR DB`.
    // Done on the user portion only (before meta tables are appended).
    content = add_nosync_markers(&content, &raw_content);

    // Process sp00ky config/backends
    let mut backend_processor = BackendProcessor::new();
    if let Some(config_path) = &config.config_path {
        if config_path.exists() {
            backend_processor.process(config_path)?;
            content.push('\n');
            content.push_str(&backend_processor.schema_appends);
            // Platform job fields on outbox tables (see
            // build_outbox_platform_fields for why).
            let tables = outbox_tables(&backend_processor);
            content.push('\n');
            content.push_str(&build_outbox_platform_fields(
                tables.iter().map(String::as_str),
            ));
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
        let endpoint = config.endpoint.as_deref().unwrap_or(default_endpoint);
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

/// Build the per-table sp00ky ingest/versioning events (`_00_<table>_mutation`
/// and `_00_<table>_delete`) for the server schema.
///
/// These events are what make change-detection work: on every mutation
/// SurrealDB `http::post($sp00ky_endpoint + '/ingest', …)`, which steps the
/// SSP's circuit and produces the `_00_list_ref` deltas clients live on.
/// Without them nothing reaches the SSP incrementally — realtime dies and
/// data only appears when a client re-registers its query (a reload).
///
/// The VM path applies these via `migrate::apply_internal_schema`, but the
/// free/Cloudflare `push` path (`build_server_schema`) does NOT emit them, so
/// `spky push` must append this. Uses `DEFINE EVENT OVERWRITE`, so it is safe
/// to (re-)apply to an existing database.
pub fn build_server_events(config: &SchemaBuilderConfig) -> Result<String> {
    let raw_content = fs::read_to_string(&config.input_path).context(format!(
        "Failed to read input schema file: {:?}",
        config.input_path
    ))?;
    // Match `build_server_schema`'s user-portion transforms so parsed table
    // shapes agree (CRDT field rewrite + backend-appended tables like the
    // outbox `job` table).
    let mut content = rewrite_crdt_cursor_fields(&raw_content);
    let mut backend_processor = BackendProcessor::new();
    if let Some(config_path) = &config.config_path {
        if config_path.exists() {
            backend_processor.process(config_path)?;
            content.push('\n');
            content.push_str(&backend_processor.schema_appends);
        }
    }

    let mut parser = SchemaParser::new();
    parser
        .parse_file(&content)
        .context("Failed to parse schema for sp00ky event generation")?;

    Ok(crate::sp00ky::generate_sp00ky_events(
        &parser.tables,
        &content,
        false, // is_client = false (server-side events)
        &config.mode,
        config.endpoint.as_deref(),
        config.secret.as_deref(),
    ))
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

/// Rewrite each `DEFINE FIELD` line whose annotations include both
/// `@crdt` and `@cursor` so its TYPE clause becomes
/// `option<object> FLEXIBLE`. Other lines pass through unchanged. This is
/// applied to both client (in `main.rs::filter_schema_for_client`) and
/// server (in `build_server_schema`) so the two schemas agree on the
/// field's storage shape.
pub fn rewrite_crdt_cursor_fields(content: &str) -> String {
    let field_annotations = annotations::extract_field_annotations(content);
    let define_field_re = Regex::new(
        r"(?i)DEFINE\s+FIELD\s+(?:OVERWRITE\s+|IF\s+NOT\s+EXISTS\s+)?(\w+)\s+ON\s+(?:TABLE\s+)?(\w+)",
    )
    .expect("static regex");
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Some(caps) = define_field_re.captures(line) {
            let field_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let table_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if let Some(anns) =
                field_annotations.get(&(table_name.to_string(), field_name.to_string()))
            {
                if let Some(rewritten) = annotations::rewrite_crdt_cursor_type(line, anns) {
                    out_lines.push(rewritten);
                    continue;
                }
            }
        }
        out_lines.push(line.to_string());
    }
    out_lines.join("\n")
}

/// Append `COMMENT 'sp00ky:nosync'` to the `DEFINE TABLE` statement of every
/// table marked `-- @nosync` in `source`. The marker is read back from
/// `INFO FOR DB` by the scheduler and SSP to exclude the table from sync.
/// If a table already carries a `COMMENT`, the marker token is merged into the
/// existing comment text rather than emitting a second (invalid) COMMENT clause.
pub fn add_nosync_markers(content: &str, source: &str) -> String {
    let nosync: std::collections::HashSet<String> = annotations::extract_table_annotations(source)
        .into_iter()
        .filter(|(_, anns)| anns.iter().any(|a| a.name == "nosync"))
        .map(|(t, _)| t)
        .collect();
    if nosync.is_empty() {
        return content.to_string();
    }

    let define_table_re =
        Regex::new(r"(?i)^\s*DEFINE\s+TABLE\s+(?:OVERWRITE\s+|IF\s+NOT\s+EXISTS\s+)?(\w+)")
            .expect("static regex");

    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let is_nosync_table = define_table_re
            .captures(line)
            .map(|c| nosync.contains(&c[1]))
            .unwrap_or(false);

        if is_nosync_table {
            // Accumulate the (possibly multi-line) statement up to its `;`.
            let mut stmt: Vec<String> = Vec::new();
            let mut found_semi = false;
            while i < lines.len() {
                stmt.push(lines[i].to_string());
                if lines[i].contains(';') {
                    found_semi = true;
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(inject_nosync_comment(&stmt.join("\n")));
            let _ = found_semi;
            continue;
        }

        out.push(line.to_string());
        i += 1;
    }
    out.join("\n")
}

/// Insert/merge the `sp00ky:nosync` marker into a single `DEFINE TABLE`
/// statement string.
fn inject_nosync_comment(stmt: &str) -> String {
    let comment_re = Regex::new(r#"(?is)COMMENT\s+(['"])(.*?)['"]"#).expect("static regex");
    if let Some(caps) = comment_re.captures(stmt) {
        let whole = caps.get(0).unwrap();
        let quote = &caps[1];
        let existing = &caps[2];
        if existing.contains(ssp_protocol::NOSYNC_TABLE_COMMENT) {
            return stmt.to_string();
        }
        let merged = format!(
            "COMMENT {q}{existing} {marker}{q}",
            q = quote,
            existing = existing,
            marker = ssp_protocol::NOSYNC_TABLE_COMMENT
        );
        let mut s = String::with_capacity(stmt.len() + merged.len());
        s.push_str(&stmt[..whole.start()]);
        s.push_str(&merged);
        s.push_str(&stmt[whole.end()..]);
        return s;
    }

    // No existing COMMENT — insert before the terminating ';'.
    let marker = format!("COMMENT '{}'", ssp_protocol::NOSYNC_TABLE_COMMENT);
    if let Some(pos) = stmt.rfind(';') {
        let (head, tail) = stmt.split_at(pos);
        format!("{} {}{}", head.trim_end(), marker, tail)
    } else {
        format!("{} {}", stmt.trim_end(), marker)
    }
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
    out = parent_id_re
        .replace_all(&out, "$$parent.record_id")
        .to_string();

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
    fn nosync_marker_added_to_marked_table_only() {
        let src = "-- @nosync\nDEFINE TABLE secrets SCHEMALESS PERMISSIONS FOR select WHERE false;\nDEFINE TABLE public SCHEMALESS;\n";
        let out = add_nosync_markers(src, src);
        let secrets = out.lines().find(|l| l.contains("TABLE secrets")).unwrap();
        assert!(
            secrets.contains("COMMENT 'sp00ky:nosync'"),
            "got: {secrets}"
        );
        assert!(
            secrets.trim_end().ends_with(';'),
            "marker before semicolon: {secrets}"
        );
        let public = out.lines().find(|l| l.contains("TABLE public")).unwrap();
        assert!(!public.contains("sp00ky:nosync"));
    }

    #[test]
    fn nosync_marker_merges_existing_comment() {
        let src = "-- @nosync\nDEFINE TABLE secrets SCHEMALESS COMMENT 'sensitive';\n";
        let out = add_nosync_markers(src, src);
        // Single COMMENT clause containing both texts.
        assert_eq!(
            out.matches("COMMENT").count(),
            1,
            "exactly one COMMENT: {out}"
        );
        assert!(out.contains("sensitive"));
        assert!(out.contains("sp00ky:nosync"));
    }

    #[test]
    fn nosync_marker_idempotent() {
        let src = "-- @nosync\nDEFINE TABLE secrets SCHEMALESS;\n";
        let once = add_nosync_markers(src, src);
        let twice = add_nosync_markers(&once, src);
        assert_eq!(once, twice);
        assert_eq!(once.matches("sp00ky:nosync").count(), 1);
    }

    #[test]
    fn no_markers_when_nothing_marked() {
        let src = "DEFINE TABLE a SCHEMALESS;\nDEFINE TABLE b SCHEMALESS;\n";
        assert_eq!(add_nosync_markers(src, src), src.to_string());
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

    fn write_tmp_schema(body: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "spky_schema_test_{}_{}.surql",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&p, body).unwrap();
        p
    }

    // Regression: the free/Cloudflare `push` path must ship the per-table
    // ingest events. Without them SurrealDB never POSTs mutations to the SSP's
    // `/ingest`, so no `_00_list_ref` deltas are produced and realtime silently
    // dies (data only reappears on a client re-register / page reload). This is
    // the exact production bug on the free plan.
    #[test]
    fn build_server_events_emits_ingest_events_for_http_mode() {
        let path = write_tmp_schema(
            "DEFINE TABLE thread SCHEMAFULL;\n\
             DEFINE FIELD title ON thread TYPE string;\n\
             DEFINE FIELD author ON thread TYPE record<user>;\n\
             DEFINE TABLE comment SCHEMAFULL;\n\
             DEFINE FIELD thread ON comment TYPE record<thread>;\n",
        );
        let config = SchemaBuilderConfig {
            input_path: path.clone(),
            config_path: None,
            mode: DeployMode::Singlenode,
            endpoint: None,
            secret: None,
            include_functions: false,
        };
        let events = build_server_events(&config).unwrap();
        std::fs::remove_file(&path).ok();

        for table in ["thread", "comment"] {
            assert!(
                events.contains(&format!("DEFINE EVENT OVERWRITE _00_{table}_mutation")),
                "missing {table} mutation event in:\n{events}"
            );
            assert!(
                events.contains(&format!("DEFINE EVENT OVERWRITE _00_{table}_delete")),
                "missing {table} delete event in:\n{events}"
            );
        }
        assert!(
            events.contains("http::post($sp00ky_endpoint + '/ingest'"),
            "ingest events must POST to the SSP's /ingest (realtime depends on it):\n{events}"
        );
    }

    // The whole pushed schema (what `spky push` sends the free node) must carry
    // the ingest events, not just the meta tables/functions.
    #[test]
    fn pushed_free_plan_schema_includes_ingest_events() {
        let path = write_tmp_schema(
            "DEFINE TABLE thread SCHEMAFULL;\n\
             DEFINE FIELD title ON thread TYPE string;\n",
        );
        let config = SchemaBuilderConfig {
            input_path: path.clone(),
            config_path: None,
            mode: DeployMode::Singlenode,
            endpoint: None,
            secret: None,
            include_functions: false,
        };
        // Mirror `push_schema_inner`: server schema + appended ingest events.
        let mut sql = build_server_schema(&config).unwrap();
        sql.push('\n');
        sql.push_str(&build_server_events(&config).unwrap());
        std::fs::remove_file(&path).ok();

        assert!(
            sql.contains("DEFINE EVENT OVERWRITE _00_thread_mutation")
                && sql.contains("http::post($sp00ky_endpoint + '/ingest'"),
            "pushed free-plan schema is missing the /ingest events → realtime would be broken"
        );
    }
}

#[cfg(test)]
mod outbox_platform_field_tests {
    use super::build_outbox_platform_fields;

    #[test]
    fn emits_if_not_exists_assignee_per_table() {
        let sql = build_outbox_platform_fields(["job", "statistics_job"]);
        assert_eq!(sql.matches("DEFINE FIELD IF NOT EXISTS assignee ON ").count(), 2);
        assert!(sql.contains("ON job TYPE option<string>"));
        assert!(sql.contains("ON statistics_job TYPE option<string>"));
        // Clients must never write the claim marker.
        assert!(sql.contains("FOR create, update WHERE false"));
        // OVERWRITE would clobber a user's own definition — must not appear.
        assert!(!sql.contains("OVERWRITE"));
    }

    #[test]
    fn empty_input_emits_nothing() {
        assert!(build_outbox_platform_fields([]).is_empty());
        assert!(build_outbox_platform_fields([""]).is_empty());
    }
}
