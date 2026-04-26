//! `spky doctor` — fast structured diagnostics for a sp00ky project.
//!
//! Designed as the agent feedback loop after a schema or config edit:
//! every check is filesystem-only by default (sub-second), every result
//! has a `fix` hint that the agent can act on, and `--json` emits a
//! stable contract that LLMs can grep.
//!
//! Heavier checks (DB reachability, migration drift via DB) are gated
//! behind explicit flags so the default invocation stays fast.
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::backend::{ClientFormat, Sp00kyConfig, DEFAULT_CONFIG_PATH};

const PREFIX: &str = "[sp00ky doctor]";

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub fix: Option<String>,
    pub detail: Option<String>,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<Option<String>>) -> Self {
        Self { name, ok: true, fix: None, detail: detail.into() }
    }
    fn fail(name: &'static str, fix: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            fix: Some(fix.into()),
            detail: Some(detail.into()),
        }
    }
    fn to_json(&self) -> Value {
        let mut obj = json!({ "name": self.name, "ok": self.ok });
        if let Some(fix) = &self.fix {
            obj["fix"] = json!(fix);
        }
        if let Some(detail) = &self.detail {
            obj["detail"] = json!(detail);
        }
        obj
    }
}

pub fn run(emit_json: bool, project_dir: &Path) -> Result<()> {
    let mut checks: Vec<Check> = Vec::new();

    // 1. Config existence + parse
    let config_path = project_dir.join(DEFAULT_CONFIG_PATH);
    let config = match check_config(&config_path) {
        Ok((c, check)) => {
            checks.push(check);
            Some(c)
        }
        Err(check) => {
            checks.push(check);
            None
        }
    };

    // 2. Schema source files exist
    if let Some(cfg) = config.as_ref() {
        checks.push(check_schema_source(cfg, project_dir));
        // 3. Codegen freshness for each clientTypes output
        checks.extend(check_codegen_freshness(cfg, project_dir));
        // 4. Migrations dir presence (informational — not a hard failure)
        checks.push(check_migrations_dir(cfg, project_dir));
    }

    let ok = checks.iter().all(|c| c.ok);

    if emit_json {
        let payload = json!({
            "ok": ok,
            "checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human(&checks, ok);
    }

    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn check_config(config_path: &Path) -> std::result::Result<(Sp00kyConfig, Check), Check> {
    if !config_path.exists() {
        return Err(Check::fail(
            "config",
            "spky create  # scaffold sp00ky.yml",
            format!("{} not found", config_path.display()),
        ));
    }
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            return Err(Check::fail(
                "config",
                "fix file permissions on sp00ky.yml",
                format!("{}: {}", config_path.display(), e),
            ));
        }
    };
    match serde_yaml::from_str::<Sp00kyConfig>(&content) {
        Ok(cfg) => Ok((
            cfg,
            Check::pass("config", Some(format!("{}", config_path.display()))),
        )),
        Err(e) => Err(Check::fail(
            "config",
            "edit sp00ky.yml — see error detail for line/column",
            format!("yaml parse error: {}", e),
        )),
    }
}

fn check_schema_source(cfg: &Sp00kyConfig, project_dir: &Path) -> Check {
    let resolved = crate::backend::ResolvedSchema::from_config(&cfg.schema);
    let schema_path = project_dir.join(&resolved.schema);
    if schema_path.exists() {
        Check::pass(
            "schema-source",
            Some(format!("{}", schema_path.display())),
        )
    } else {
        Check::fail(
            "schema-source",
            format!("create {}", schema_path.display()),
            format!(
                "schema source not found at {} (configured via `schema:` in sp00ky.yml)",
                schema_path.display()
            ),
        )
    }
}

/// For each `clientTypes` entry: does the output exist, and is its mtime
/// at least as new as the latest `.surql` file under the schema dir?
fn check_codegen_freshness(cfg: &Sp00kyConfig, project_dir: &Path) -> Vec<Check> {
    if cfg.client_types.is_empty() {
        return vec![Check::pass(
            "codegen-fresh",
            Some("no clientTypes configured".to_string()),
        )];
    }

    let resolved = crate::backend::ResolvedSchema::from_config(&cfg.schema);
    let schema_root = project_dir.join(resolved.schema.parent().unwrap_or_else(|| Path::new(".")));
    let newest_source = newest_surql_mtime(&schema_root);

    let mut out = Vec::new();
    for ct in &cfg.client_types {
        let label: &'static str = match ct.format {
            ClientFormat::Typescript => "codegen-fresh:ts",
            ClientFormat::Dart => "codegen-fresh:dart",
        };
        let output_path = project_dir.join(&ct.output);
        if !output_path.exists() {
            out.push(Check::fail(
                label,
                "spky generate".to_string(),
                format!("output missing: {}", output_path.display()),
            ));
            continue;
        }
        let output_mtime = mtime(&output_path);
        match (output_mtime, newest_source) {
            (Some(out_t), Some(src_t)) if out_t < src_t => out.push(Check::fail(
                label,
                "spky generate".to_string(),
                format!(
                    "{} is older than the newest .surql under {}",
                    ct.output,
                    schema_root.display()
                ),
            )),
            _ => out.push(Check::pass(
                label,
                Some(format!("{}", output_path.display())),
            )),
        }
    }
    out
}

fn check_migrations_dir(cfg: &Sp00kyConfig, project_dir: &Path) -> Check {
    let resolved = crate::backend::ResolvedSchema::from_config(&cfg.schema);
    let path = project_dir.join(&resolved.migrations);
    if path.exists() {
        let count = fs::read_dir(&path)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|x| x == "surql")
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        Check::pass(
            "migrations-dir",
            Some(format!("{} ({} migration file(s))", path.display(), count)),
        )
    } else {
        Check::fail(
            "migrations-dir",
            format!("mkdir -p {}", path.display()),
            format!("{} does not exist", path.display()),
        )
    }
}

fn newest_surql_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    walk(dir, &mut |p| {
        if p.extension().map(|e| e == "surql").unwrap_or(false) {
            if let Some(t) = mtime(p) {
                newest = Some(match newest {
                    Some(n) if n > t => n,
                    _ => t,
                });
            }
        }
    });
    newest
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn print_human(checks: &[Check], ok: bool) {
    println!();
    for c in checks {
        let mark = if c.ok { "✓" } else { "✗" };
        println!("  {} {}", mark, c.name);
        if let Some(detail) = &c.detail {
            println!("      {}", detail);
        }
        if let Some(fix) = &c.fix {
            println!("      fix: {}", fix);
        }
    }
    println!();
    if ok {
        println!("{} OK", PREFIX);
    } else {
        println!("{} {} check(s) failed", PREFIX, checks.iter().filter(|c| !c.ok).count());
    }
}

#[allow(dead_code)]
pub fn _unused(_: PathBuf) {} // keep PathBuf import warning quiet across feature flags
