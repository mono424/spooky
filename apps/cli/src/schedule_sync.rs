//! Push `schedules:` / `workflows:` from the manifest into `_00_schedule`.
//!
//! Runs right after the internal schema is applied, on both the deploy and the
//! `spky dev` path, because that is where the CLI already holds a root
//! connection. The rows are the engine's only source of truth — nothing reads
//! sp00ky.yml at runtime.
//!
//! Desired-state, with one hard rule: **deploy writes spec fields and nothing
//! else.** `paused` belongs to the operator and `next_fire_at` / `last_*` to the
//! engine, so a redeploy must not resurrect a schedule someone paused, or reset a
//! clock mid-cycle. The only exception is deliberate: when the spec hash changes,
//! `next_fire_at` is cleared so the engine replans against the new cadence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::backend::{AppScope, AppType, BackendProcessor, Sp00kyConfig};
use crate::migrate::checksum_str;
use crate::schedule_config::{normalize_schedule, normalize_workflow};
use crate::surreal_client::MigrationDB;

/// What one sync pass did, for the CLI's output.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
}

impl SyncReport {
    pub fn is_noop(&self) -> bool {
        self.created == 0 && self.updated == 0 && self.removed == 0
    }

    pub fn summary(&self) -> String {
        format!(
            "{} created, {} updated, {} unchanged, {} removed",
            self.created, self.updated, self.unchanged, self.removed
        )
    }
}

/// Resolve every schedule/workflow to the row the engine reads.
///
/// Returns `(name, spec_row)` pairs. Fails when a definition names a backend or
/// route that doesn't exist — the same check `spky lint` makes, repeated here so
/// a deploy can't write a schedule that could never run.
pub fn resolve_rows(
    config: &Sp00kyConfig,
    processor: &BackendProcessor,
    base_dir: &Path,
) -> Result<Vec<(String, Value)>> {
    let mut rows = Vec::new();

    for (name, cfg) in &config.schedules {
        let label = format!("schedule '{name}'");
        let table = outbox_table_for(config, &cfg.backend, &label)?;
        check_route(config, processor, base_dir, &cfg.backend, &cfg.route, &label)?;
        rows.push((name.clone(), normalize_schedule(name, cfg, &table)?));
    }

    for (name, cfg) in &config.workflows {
        // A workflow's default table comes from whichever backend its first step
        // uses; every step also carries its own when it differs.
        let mut step_tables = BTreeMap::new();
        for (step_name, step) in &cfg.steps {
            let label = format!("workflow '{name}' step '{step_name}'");
            let table = outbox_table_for(config, &step.backend, &label)?;
            check_route(config, processor, base_dir, &step.backend, &step.route, &label)?;
            step_tables.insert(step_name.clone(), table);
        }
        let default_table = cfg
            .steps
            .keys()
            .next()
            .and_then(|first| step_tables.get(first).cloned())
            .unwrap_or_default();
        rows.push((
            name.clone(),
            normalize_workflow(name, cfg, &default_table, &step_tables)?,
        ));
    }

    Ok(rows)
}

/// The outbox table a backend app writes jobs to.
///
/// Read from `apps.<name>.method.table` rather than from `BackendProcessor`,
/// because the processor deliberately skips `scope: devOnly` backends (they carry
/// no schema to apply) — and a dev-only backend is still a perfectly good target
/// for a schedule you run locally.
fn outbox_table_for(config: &Sp00kyConfig, backend: &str, label: &str) -> Result<String> {
    let app = config.apps.get(backend).with_context(|| {
        format!("{label} references backend '{backend}', which is not an app in sp00ky.yml")
    })?;
    if app.app_type != AppType::Backend {
        anyhow::bail!("{label} references '{backend}', which is not a backend app");
    }
    app.method
        .as_ref()
        .and_then(|m| m.table.clone())
        .with_context(|| {
            format!(
                "{label} references backend '{backend}', which has no outbox table \
                 (add `method: {{ type: outbox, table: … }}`)"
            )
        })
}

/// Backends a schedule targets that won't exist in a cloud deployment.
///
/// A `devOnly` backend is never deployed, so its schedule can fire locally but
/// would spawn jobs with nowhere to dispatch them in the cloud. Surfaced as a
/// lint warning rather than an error, because running one locally is legitimate.
pub fn dev_only_targets(config: &Sp00kyConfig) -> Vec<String> {
    let dev_only = |backend: &str| {
        config
            .apps
            .get(backend)
            .is_some_and(|app| app.scope == AppScope::DevOnly)
    };
    let mut out = Vec::new();
    for (name, cfg) in &config.schedules {
        if dev_only(&cfg.backend) {
            out.push(format!("schedule '{name}' targets dev-only backend '{}'", cfg.backend));
        }
    }
    for (name, cfg) in &config.workflows {
        for (step, s) in &cfg.steps {
            if dev_only(&s.backend) {
                out.push(format!(
                    "workflow '{name}' step '{step}' targets dev-only backend '{}'",
                    s.backend
                ));
            }
        }
    }
    out
}

fn check_route(
    config: &Sp00kyConfig,
    processor: &BackendProcessor,
    base_dir: &Path,
    backend: &str,
    route: &str,
    label: &str,
) -> Result<()> {
    // Prefer the processor's map (it already parsed the spec), and fall back to
    // reading the spec directly for backends it skipped — `devOnly` ones, which are
    // exactly the backends a locally-run schedule targets.
    let known: BTreeSet<String> = match processor.backend_definitions.get(backend) {
        Some(def) if !def.routes.is_empty() => def.routes.keys().cloned().collect(),
        _ => match declared_post_routes(config, base_dir, backend) {
            Some(routes) if !routes.is_empty() => routes,
            // No spec to check against: unknown, not empty. Don't reject on
            // missing information.
            _ => return Ok(()),
        },
    };

    if known.contains(route) {
        return Ok(());
    }
    anyhow::bail!(
        "{label} calls '{route}', which backend '{backend}' does not define. Known routes: {}",
        known.iter().cloned().collect::<Vec<_>>().join(", ")
    )
}

/// POST paths declared by a backend's OpenAPI spec.
///
/// A deliberately minimal read: schedules only need to know whether a path
/// exists, while `BackendProcessor` additionally types each route's arguments for
/// codegen. `None` when the app has no spec or it can't be read.
fn declared_post_routes(
    config: &Sp00kyConfig,
    base_dir: &Path,
    backend: &str,
) -> Option<BTreeSet<String>> {
    let spec_rel = config.apps.get(backend)?.spec.as_ref()?;
    let content = std::fs::read_to_string(base_dir.join(spec_rel)).ok()?;
    let spec: openapiv3::OpenAPI = serde_yaml::from_str(&content).ok()?;
    Some(
        spec.paths
            .paths
            .into_iter()
            .filter(|(_, item)| item.as_item().is_some_and(|i| i.post.is_some()))
            .map(|(path, _)| path)
            .collect(),
    )
}

/// Upsert the definitions and delete the ones that are gone.
pub fn sync(
    client: &dyn MigrationDB,
    config: &Sp00kyConfig,
    processor: &BackendProcessor,
    base_dir: &Path,
) -> Result<SyncReport> {
    let rows = resolve_rows(config, processor, base_dir)?;
    let mut report = SyncReport::default();

    let stored = read_stored_hashes(client)?;

    for (name, spec) in &rows {
        let canonical = serde_json::to_string(spec)?;
        let hash = checksum_str(&canonical);
        let existed = stored.contains_key(name);
        if stored.get(name).map(String::as_str) == Some(hash.as_str()) {
            report.unchanged += 1;
            continue;
        }

        // MERGE, not CONTENT: engine and operator fields on this row must
        // survive. And `next_fire_at = NONE` only here, on a real spec change,
        // so the engine replans the new cadence instead of firing on the old one.
        let sql = format!(
            "UPSERT {id} MERGE {patch}; UPDATE {id} SET next_fire_at = NONE;",
            id = record_literal(name),
            patch = merge_patch(spec, &hash)?,
        );
        client
            .execute(&sql)
            .with_context(|| format!("failed to sync schedule '{name}'"))?;

        if existed {
            report.updated += 1;
        } else {
            report.created += 1;
        }
    }

    // Sweep: anything config-owned that is no longer declared. Run history keeps
    // its own denormalized `schedule_name`, so it outlives the definition.
    let declared: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
    for name in stored.keys() {
        if !declared.contains(&name.as_str()) {
            client
                .execute(&format!("DELETE {};", record_literal(name)))
                .with_context(|| format!("failed to remove schedule '{name}'"))?;
            report.removed += 1;
        }
    }

    Ok(report)
}

/// `name -> spec_hash` for every config-owned schedule currently in the DB.
fn read_stored_hashes(client: &dyn MigrationDB) -> Result<BTreeMap<String, String>> {
    let responses = client
        .execute("SELECT name, spec_hash FROM _00_schedule;")
        .context("failed to read existing schedules")?;
    let mut out = BTreeMap::new();
    for response in responses {
        let Some(result) = response.result else { continue };
        let rows = match result {
            Value::Array(rows) => rows,
            other => vec![other],
        };
        for row in rows {
            let Some(name) = row.get("name").and_then(Value::as_str) else { continue };
            let hash = row.get("spec_hash").and_then(Value::as_str).unwrap_or_default();
            out.insert(name.to_string(), hash.to_string());
        }
    }
    Ok(out)
}

/// The `MERGE` object: spec fields plus the hash, and nothing else.
fn merge_patch(spec: &Value, hash: &str) -> Result<String> {
    let mut patch = spec.as_object().cloned().unwrap_or_default();
    patch.insert("spec_hash".into(), Value::String(hash.to_string()));
    debug_assert!(
        !patch.contains_key("paused")
            && !patch.contains_key("next_fire_at")
            && !patch.contains_key("last_fire_at")
            && !patch.contains_key("trigger_requested_at"),
        "deploy must never write operator- or engine-owned fields"
    );
    Ok(serde_json::to_string(&Value::Object(patch))?)
}

/// `_00_schedule:⟨name⟩`. The ⟨⟩ form quotes any key, which matters because
/// schedule names routinely contain hyphens.
pub fn record_literal(name: &str) -> String {
    format!("_00_schedule:⟨{}⟩", name.replace('⟩', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_literals_quote_hyphenated_names() {
        assert_eq!(record_literal("game-sync"), "_00_schedule:⟨game-sync⟩");
    }

    /// The whole point of MERGE: a deploy must not touch operator or engine state.
    #[test]
    fn the_merge_patch_carries_only_spec_fields() {
        let spec = serde_json::json!({
            "name": "nightly",
            "kind": "job",
            "cron": "0 3 * * *",
            "target_table": "job",
            "path": "/cleanup",
        });
        let patch = merge_patch(&spec, "abc123").unwrap();
        assert!(patch.contains("\"spec_hash\":\"abc123\""));
        for forbidden in ["paused", "next_fire_at", "last_fire_at", "trigger_requested_at"] {
            assert!(!patch.contains(forbidden), "patch must not mention {forbidden}");
        }
    }

    #[test]
    fn report_summarizes_and_detects_a_noop() {
        let mut report = SyncReport::default();
        report.unchanged = 3;
        assert!(report.is_noop());
        report.updated = 1;
        assert!(!report.is_noop());
        assert_eq!(report.summary(), "0 created, 1 updated, 3 unchanged, 0 removed");
    }
}
