use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::backend::{self, BackendDevConfig, BackendDevTypedConfig, HostingMode};
use crate::surreal_client::MigrationDB;
use crate::{CloudBackupCommands, CloudBillingCommands, CloudCommands, CloudKeyCommands, CloudLinkCommands};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const DEFAULT_API_URL: &str = "https://api.sp00ky.cloud";
const CREDENTIALS_FILE: &str = "credentials.json";
const SP00KY_DIR: &str = ".sp00ky";

fn api_base_url() -> String {
    // Priority: env var > sp00ky.yml cloudApi > default
    if let Ok(url) = std::env::var("SP00KY_CLOUD_API") {
        return url;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let config = backend::load_config(&cwd.join("sp00ky.yml"));
        if let Some(url) = config.cloud_api {
            return url;
        }
    }
    DEFAULT_API_URL.to_string()
}

/// Derive the SurrealDB WebSocket endpoint for a project.
/// api-staging.sp00ky.cloud → wss://{slug}.db.staging.spky.cloud/rpc
/// api.sp00ky.cloud → wss://{slug}.db.spky.cloud/rpc
fn derive_db_ws_endpoint(api_url: &str, slug: &str) -> Option<String> {
    let host = api_url.strip_prefix("https://")
        .or(api_url.strip_prefix("http://"))?
        .split('/').next()?;

    if let Some(stage) = host.strip_prefix("api-").and_then(|h| h.strip_suffix(".sp00ky.cloud")) {
        // Staging: api-staging.sp00ky.cloud → db.staging.spky.cloud
        Some(format!("wss://{}.db.{}.spky.cloud/rpc", slug, stage))
    } else if host.ends_with(".sp00ky.cloud") {
        // Production: api.sp00ky.cloud → db.spky.cloud
        Some(format!("wss://{}.db.spky.cloud/rpc", slug))
    } else {
        None
    }
}

/// Derive the upload URL for large files (bypasses Cloudflare proxy).
/// Converts "https://api-staging.sp00ky.cloud" → "https://upload.staging.spky.cloud"
/// Falls back to the API URL if no upload domain can be derived.
fn upload_base_url(api_url: &str) -> String {
    if let Ok(url) = std::env::var("SP00KY_CLOUD_UPLOAD") {
        return url;
    }
    // Try to derive: api-{stage}.sp00ky.cloud → upload.{stage}.spky.cloud
    if let Some(host) = api_url.strip_prefix("https://").or(api_url.strip_prefix("http://")) {
        let host = host.split('/').next().unwrap_or(host);
        if host.contains("sp00ky.cloud") {
            return "https://upload.spky.cloud".to_string();
        }
    }
    api_url.to_string()
}

fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(SP00KY_DIR)
        .join(CREDENTIALS_FILE)
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct Credentials {
    access_token: String,
    refresh_token: String,
}

fn load_credentials() -> Option<Credentials> {
    // API key from environment takes precedence (for CI/CD)
    if let Ok(api_key) = std::env::var("SPOOKY_API_KEY") {
        if api_key.starts_with("spk_live_") {
            return Some(Credentials {
                access_token: api_key,
                refresh_token: String::new(),
            });
        }
    }
    let path = credentials_path();
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_credentials(creds: &Credentials) -> Result<()> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .context("Failed to create ~/.sp00ky directory")?;
    }
    let json = serde_json::to_string_pretty(creds)?;
    fs::write(&path, json).context("Failed to write credentials")?;
    Ok(())
}

fn clear_credentials() -> Result<()> {
    let path = credentials_path();
    if path.exists() {
        fs::remove_file(&path).context("Failed to remove credentials file")?;
    }
    Ok(())
}

fn require_credentials() -> Result<Credentials> {
    load_credentials().ok_or_else(|| {
        anyhow::anyhow!("Not logged in. Run `sp00ky cloud login` first.")
    })
}

// ---------------------------------------------------------------------------
// Cloud HTTP Client
// ---------------------------------------------------------------------------

struct CloudClient {
    base_url: String,
    token: String,
    refresh_token: String,
    is_api_key: bool,
}

impl CloudClient {
    fn new(creds: &Credentials) -> Self {
        let is_api_key = creds.access_token.starts_with("spk_live_");
        Self {
            base_url: api_base_url(),
            token: creds.access_token.clone(),
            refresh_token: creds.refresh_token.clone(),
            is_api_key,
        }
    }

    fn auth_header(&self) -> String {
        if self.is_api_key {
            format!("ApiKey {}", self.token)
        } else {
            format!("Bearer {}", self.token)
        }
    }

    fn try_refresh(&mut self) -> Result<()> {
        let url = format!("{}/v1/auth/refresh", self.base_url);
        let body = serde_json::json!({ "refresh_token": self.refresh_token });
        let resp = ureq::post(&url)
            .set("Accept", "application/json")
            .send_json(body)
            .map_err(|e| anyhow::anyhow!("Token refresh failed: {}", e))?;
        let tokens: serde_json::Value = resp.into_json().context("Failed to parse refresh response")?;
        let access = tokens["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token in refresh response"))?;
        let refresh = tokens["refresh_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing refresh_token in refresh response"))?;
        self.token = access.to_string();
        self.refresh_token = refresh.to_string();
        save_credentials(&Credentials {
            access_token: self.token.clone(),
            refresh_token: self.refresh_token.clone(),
        })?;
        Ok(())
    }

    fn format_api_error(code: u16, resp: ureq::Response) -> String {
        let body = resp.into_string().unwrap_or_default();
        if body.contains("<!DOCTYPE") || body.contains("<html") {
            // HTML error page (e.g. Cloudflare 502/503)
            format!("API unavailable (HTTP {}). The server may be restarting — try again in a moment.", code)
        } else if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
            let error_code = json["code"].as_str().unwrap_or("");
            let error_msg = json["error"].as_str().unwrap_or(&body);
            match error_code {
                "project_not_active" => {
                    "Project billing is not set up. Run `sp00ky cloud billing` or `sp00ky cloud deploy` to get started.".to_string()
                }
                "plan_limit" => {
                    format!("{}. Run `sp00ky cloud billing` to manage your plan.", error_msg)
                }
                "slug_taken" => {
                    "A project with this slug already exists — choose a different slug.".to_string()
                }
                _ if !error_msg.is_empty() => {
                    format!("API error (HTTP {}): {}", code, error_msg)
                }
                _ => format!("API error (HTTP {}): {}", code, body),
            }
        } else {
            format!("API error (HTTP {}): {}", code, body)
        }
    }

    fn get(&mut self, path: &str) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
        match ureq::get(&url)
            .set("Authorization", &self.auth_header())
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) if !self.is_api_key => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::get(&url)
                    .set("Authorization", &self.auth_header())
                    .set("Accept", "application/json")
                    .call()
                {
                    Ok(resp) => Ok(resp),
                    Err(ureq::Error::Status(401, _)) => {
                        bail!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        bail!("{}", Self::format_api_error(code, resp))
                    }
                    Err(ureq::Error::Transport(t)) => {
                        bail!("Failed to connect to Sp00ky Cloud API: {}", t)
                    }
                }
            }
            Err(ureq::Error::Status(code, resp)) => {
                bail!("{}", Self::format_api_error(code, resp))
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }

    fn post(&mut self, path: &str, body: &serde_json::Value) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
        match ureq::post(&url)
            .set("Authorization", &self.auth_header())
            .set("Accept", "application/json")
            .send_json(body.clone())
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) if !self.is_api_key => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::post(&url)
                    .set("Authorization", &self.auth_header())
                    .set("Accept", "application/json")
                    .send_json(body.clone())
                {
                    Ok(resp) => Ok(resp),
                    Err(ureq::Error::Status(401, _)) => {
                        bail!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        bail!("{}", Self::format_api_error(code, resp))
                    }
                    Err(ureq::Error::Transport(t)) => {
                        bail!("Failed to connect to Sp00ky Cloud API: {}", t)
                    }
                }
            }
            Err(ureq::Error::Status(code, resp)) => {
                bail!("{}", Self::format_api_error(code, resp))
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }

    fn delete(&mut self, path: &str) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
        match ureq::delete(&url)
            .set("Authorization", &self.auth_header())
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) if !self.is_api_key => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::delete(&url)
                    .set("Authorization", &self.auth_header())
                    .set("Accept", "application/json")
                    .call()
                {
                    Ok(resp) => Ok(resp),
                    Err(ureq::Error::Status(401, _)) => {
                        bail!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        bail!("{}", Self::format_api_error(code, resp))
                    }
                    Err(ureq::Error::Transport(t)) => {
                        bail!("Failed to connect to Sp00ky Cloud API: {}", t)
                    }
                }
            }
            Err(ureq::Error::Status(code, resp)) => {
                bail!("{}", Self::format_api_error(code, resp))
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }
    fn patch(&mut self, path: &str, body: &serde_json::Value) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
        match ureq::request("PATCH", &url)
            .set("Authorization", &self.auth_header())
            .set("Accept", "application/json")
            .send_json(body.clone())
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) if !self.is_api_key => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::request("PATCH", &url)
                    .set("Authorization", &self.auth_header())
                    .set("Accept", "application/json")
                    .send_json(body.clone())
                {
                    Ok(resp) => Ok(resp),
                    Err(ureq::Error::Status(401, _)) => {
                        bail!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        bail!("{}", Self::format_api_error(code, resp))
                    }
                    Err(ureq::Error::Transport(t)) => {
                        bail!("Failed to connect to Sp00ky Cloud API: {}", t)
                    }
                }
            }
            Err(ureq::Error::Status(code, resp)) => {
                bail!("{}", Self::format_api_error(code, resp))
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Project slug resolution
// ---------------------------------------------------------------------------

fn resolve_project_slug() -> Result<String> {
    // 1. Environment variable
    if let Ok(slug) = std::env::var("SP00KY_CLOUD_PROJECT") {
        return Ok(slug);
    }

    // 2. Read slug from sp00ky.yml
    if let Ok(cwd) = std::env::current_dir() {
        let config_path = cwd.join("sp00ky.yml");
        if config_path.exists() {
            let config = backend::load_config(&config_path);
            if let Some(slug) = config.slug {
                return Ok(slug);
            }
        }
    }

    // 3. Prompt
    let slug = inquire::Text::new("Project slug:")
        .with_help_message("The slug you used when creating the cloud project")
        .prompt()
        .context("Failed to read project slug")?;

    Ok(slug)
}

// ---------------------------------------------------------------------------
// Flow helpers — composable building blocks for guided CLI flows
// ---------------------------------------------------------------------------

fn is_interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// Fetch a project by slug. Returns None if not found.
fn fetch_project(client: &mut CloudClient, slug: &str) -> Result<Option<serde_json::Value>> {
    let projects = fetch_project_list(client)?;
    Ok(projects
        .into_iter()
        .find(|p| p["slug"].as_str() == Some(slug)))
}

/// Extract the project UUID from a project JSON value.
/// Falls back to slug if id is missing (shouldn't happen).
fn project_id(project: &serde_json::Value) -> String {
    project["id"]
        .as_str()
        .unwrap_or(project["slug"].as_str().unwrap_or("unknown"))
        .to_string()
}

/// Resolve the project slug and look up its UUID.
/// Used by standalone commands (status, logs, scale, destroy).
fn resolve_project_id(client: &mut CloudClient) -> Result<(String, String)> {
    let slug = resolve_project_slug()?;
    let project = fetch_project(client, &slug)?
        .ok_or_else(|| anyhow::anyhow!("Project '{}' not found.", slug))?;
    let pid = project_id(&project);
    Ok((slug, pid))
}

/// List all projects for the authenticated user.
fn fetch_project_list(client: &mut CloudClient) -> Result<Vec<serde_json::Value>> {
    let resp = client.get("/v1/projects")?;
    let projects: Vec<serde_json::Value> = resp.into_json().context("Failed to parse projects")?;
    Ok(projects)
}

/// Ensure the user is logged in. In interactive mode, offers to log in inline.
fn ensure_login() -> Result<Credentials> {
    if let Some(creds) = load_credentials() {
        return Ok(creds);
    }

    if !is_interactive() {
        bail!("Not logged in. Run `sp00ky cloud login` first.");
    }

    println!("You're not logged in to Sp00ky Cloud.");
    let do_login = inquire::Confirm::new("Log in now?")
        .with_default(true)
        .prompt()
        .context("Failed to read confirmation")?;

    if !do_login {
        bail!("Login required. Run `sp00ky cloud login` to authenticate.");
    }

    login()?;

    load_credentials().ok_or_else(|| anyhow::anyhow!("Login succeeded but credentials not found."))
}

/// Ensure a cloud project exists and return (slug, project_json).
/// In interactive mode, guides the user through project selection or creation.
fn ensure_project(client: &mut CloudClient) -> Result<(String, serde_json::Value)> {
    // 1. Try configured slug (env var or sp00ky.yml)
    if let Ok(slug) = std::env::var("SP00KY_CLOUD_PROJECT") {
        if let Some(project) = fetch_project(client, &slug)? {
            return Ok((slug, project));
        }
        bail!("Project '{}' not found (from SP00KY_CLOUD_PROJECT).", slug);
    }

    if let Ok(cwd) = std::env::current_dir() {
        let config_path = cwd.join("sp00ky.yml");
        if config_path.exists() {
            let config = backend::load_config(&config_path);
            if let Some(slug) = config.slug {
                if let Some(project) = fetch_project(client, &slug)? {
                    return Ok((slug, project));
                }
                // Slug is configured but project doesn't exist — fall through to create
                if !is_interactive() {
                    bail!("Project '{}' not found. Run `sp00ky cloud create --slug {}` first.", slug, slug);
                }
                println!("Project '{}' (from sp00ky.yml) not found in Sp00ky Cloud.", slug);
                let do_create = inquire::Confirm::new(&format!("Create project '{}'?", slug))
                    .with_default(true)
                    .prompt()
                    .context("Failed to read confirmation")?;
                if do_create {
                    return create_project_inline(client, Some(slug));
                }
                bail!("Project required. Run `sp00ky cloud create` to create one.");
            }
        }
    }

    // 2. No slug configured — check existing projects
    if !is_interactive() {
        bail!("No project configured. Set SP00KY_CLOUD_PROJECT or add `slug` to sp00ky.yml.");
    }

    let projects = fetch_project_list(client)?;
    let active_projects: Vec<&serde_json::Value> = projects
        .iter()
        .filter(|p| p["status"].as_str() != Some("destroyed"))
        .collect();

    if active_projects.is_empty() {
        println!("No cloud projects found.");
        let do_create = inquire::Confirm::new("Create a new project?")
            .with_default(true)
            .prompt()
            .context("Failed to read confirmation")?;
        if do_create {
            return create_project_inline(client, None);
        }
        bail!("Project required. Run `sp00ky cloud create` to create one.");
    }

    if active_projects.len() == 1 {
        let slug = active_projects[0]["slug"]
            .as_str()
            .context("Missing slug in project")?
            .to_string();
        let status = active_projects[0]["status"].as_str().unwrap_or("unknown");
        println!("Found project '{}' ({}).", slug, status);
        return Ok((slug, active_projects[0].clone()));
    }

    // Multiple projects — let user pick
    let slugs: Vec<String> = active_projects
        .iter()
        .map(|p| {
            format!(
                "{} ({})",
                p["slug"].as_str().unwrap_or("?"),
                p["status"].as_str().unwrap_or("?")
            )
        })
        .collect();

    let selection = inquire::Select::new("Select a project:", slugs)
        .prompt()
        .context("Failed to read selection")?;

    // Extract slug from "slug (status)" format
    let slug = selection.split(' ').next().unwrap_or("").to_string();
    let project = fetch_project(client, &slug)?
        .context("Selected project not found")?;
    Ok((slug, project))
}

/// Create a project inline and return (slug, project_json).
/// Retries with a different slug if the chosen one is already taken.
fn create_project_inline(
    client: &mut CloudClient,
    slug: Option<String>,
) -> Result<(String, serde_json::Value)> {
    let mut slug = match slug {
        Some(s) => s,
        None => inquire::Text::new("Project slug:")
            .with_help_message("Lowercase letters, numbers, and hyphens (e.g. my-app)")
            .prompt()
            .context("Failed to read slug")?,
    };

    let plan = "starter".to_string();

    loop {
        match client.post(
            "/v1/projects",
            &serde_json::json!({ "slug": slug, "plan": plan }),
        ) {
            Ok(resp) => {
                let project: serde_json::Value =
                    resp.into_json().context("Failed to parse response")?;

                println!();
                println!("  Project created!");
                println!("  Slug:   {}", project["slug"].as_str().unwrap_or(&slug));
                println!("  Plan:   {}", project["plan"].as_str().unwrap_or(&plan));
                println!(
                    "  Status: {}",
                    project["status"].as_str().unwrap_or("pending_payment")
                );

                return Ok((slug, project));
            }
            Err(err) => {
                let msg = err.to_string();
                // Catch slug conflicts: 409 "slug already exists" (new backend)
                // or 500 "failed to create project" (old backend — likely unique constraint)
                let is_slug_conflict = msg.contains("already exists")
                    || msg.contains("slug already")
                    || msg.contains("failed to create project");
                if is_slug_conflict {
                    if !is_interactive() {
                        bail!("Slug '{}' is likely already taken. Choose a different slug.", slug);
                    }
                    println!("  Slug '{}' is likely already taken. Try a different one.", slug);
                    slug = inquire::Text::new("Project slug:")
                        .with_help_message(
                            "Lowercase letters, numbers, and hyphens (e.g. my-app)",
                        )
                        .prompt()
                        .context("Failed to read slug")?;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

/// Ensure the project has active billing. Offers to set up billing inline if interactive.
fn ensure_billing_active(
    client: &mut CloudClient,
    slug: &str,
    project: &serde_json::Value,
) -> Result<()> {
    let status = project["status"].as_str().unwrap_or("unknown");

    match status {
        "active" => Ok(()),
        "pending_payment" => {
            if !is_interactive() {
                bail!(
                    "Project '{}' requires billing setup. Run `sp00ky cloud billing` first.",
                    slug
                );
            }

            println!();
            println!("  Billing is not set up for '{}'.", slug);
            let do_billing = inquire::Confirm::new("Set up billing now?")
                .with_default(true)
                .prompt()
                .context("Failed to read confirmation")?;

            if !do_billing {
                bail!("Billing required before deploying. Run `sp00ky cloud billing` when ready.");
            }

            wait_for_billing(client, slug)
        }
        "suspended" => {
            bail!(
                "Project '{}' is suspended due to a billing issue. Run `sp00ky cloud billing` to resolve.",
                slug
            );
        }
        "destroyed" => {
            bail!("Project '{}' has been destroyed.", slug);
        }
        _ => {
            bail!("Project '{}' has unexpected status: {}", slug, status);
        }
    }
}

/// Open Stripe checkout and poll until payment completes.
fn wait_for_billing(client: &mut CloudClient, slug: &str) -> Result<()> {
    // Start checkout
    let resp = client.post(
        "/v1/billing/checkout",
        &serde_json::json!({ "project_id": slug }),
    )?;
    let data: serde_json::Value =
        resp.into_json().context("Failed to parse checkout response")?;
    let url = data["url"]
        .as_str()
        .context("No checkout URL in response")?;

    println!("  Opening Stripe checkout...");
    let _ = open::that(url);
    println!("  Waiting for payment to complete... (press Ctrl+C to cancel)");

    // Poll project status every 3 seconds, timeout after 10 minutes
    let max_attempts = 200;
    for _ in 0..max_attempts {
        thread::sleep(Duration::from_secs(3));

        if let Some(project) = fetch_project(client, slug)? {
            let status = project["status"].as_str().unwrap_or("unknown");
            if status == "active" {
                println!("  Payment confirmed! Project '{}' is now active.", slug);
                return Ok(());
            }
        }
        print!(".");
    }

    bail!(
        "Timed out waiting for payment. If you completed checkout, it may take a moment to process.\n  Run `sp00ky cloud deploy` to try again."
    );
}

/// Poll deployment until SSP VMs match the target count.
fn poll_scale_completion(client: &mut CloudClient, pid: &str, target_ssp: u32) -> Result<()> {
    println!("  Waiting for scaling to complete...");

    let max_attempts = 100; // ~5 minutes at 3s intervals
    for _ in 0..max_attempts {
        thread::sleep(Duration::from_secs(3));

        let resp = client.get(&format!("/v1/projects/{}/deployment", pid));
        if let Ok(resp) = resp {
            let data: serde_json::Value = resp.into_json().unwrap_or_default();
            if let Some(vms) = data.get("vms").and_then(|v| v.as_array()) {
                let running_ssp = vms
                    .iter()
                    .filter(|vm| {
                        vm["role"].as_str() == Some("ssp")
                            && vm["status"].as_str() == Some("running")
                    })
                    .count() as u32;

                if running_ssp >= target_ssp {
                    println!("  Scaling complete! {} SSP instance(s) running.", running_ssp);
                    print_deployment_details(&data);
                    return Ok(());
                }
                print!("  SSP instances: {}/{}...\r", running_ssp, target_ssp);
            }
        }
    }

    println!();
    println!(
        "  Scaling is still in progress. Run `sp00ky cloud status` to check."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

pub fn run(action: CloudCommands) -> Result<()> {
    match action {
        CloudCommands::Login => login(),
        CloudCommands::Logout => logout(),
        CloudCommands::Create { slug, plan } => create(slug, plan),
        CloudCommands::List => list(),
        CloudCommands::Deploy => deploy(),
        CloudCommands::Status => status(),
        CloudCommands::Logs { service } => logs(service),
        CloudCommands::Scale { ssp } => scale(ssp),
        CloudCommands::Destroy => destroy(),
        CloudCommands::Upgrade { check, version, force } => upgrade(check, version, force),
        CloudCommands::Backup { action } => backup(action),
        CloudCommands::Billing { action } => billing(action),
        CloudCommands::Keys { action } => keys(action),
        CloudCommands::Link { action } => link(action),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn login() -> Result<()> {
    let base_url = api_base_url();
    let url = format!("{}/v1/auth/login", base_url);

    println!("Initiating login...");

    let resp: serde_json::Value = match ureq::post(&url)
        .set("Accept", "application/json")
        .send_json(serde_json::json!({}))
    {
        Ok(resp) => resp.into_json().context("Failed to parse login response")?,
        Err(ureq::Error::Status(code, resp)) => {
            bail!("{}", CloudClient::format_api_error(code, resp));
        }
        Err(ureq::Error::Transport(t)) => {
            bail!("Failed to connect to Sp00ky Cloud API: {}", t);
        }
    };

    let device_code = resp["device_code"]
        .as_str()
        .context("Missing device_code in response")?;
    let user_code = resp["user_code"]
        .as_str()
        .context("Missing user_code in response")?;
    let verification_url = resp["verification_url"]
        .as_str()
        .context("Missing verification_url in response")?;
    let interval = resp["interval"].as_u64().unwrap_or(5);

    let verification_url_with_code = format!("{}?code={}", verification_url, user_code);

    println!();
    println!("  Your verification code is: {}", user_code);
    println!();
    println!("  Opening browser to: {}", verification_url_with_code);
    println!("  (If the browser doesn't open, visit the URL manually)");
    println!();

    // Open browser
    let _ = open::that(&verification_url_with_code);

    // Poll for completion
    let poll_url = format!("{}/v1/auth/login/poll", base_url);

    loop {
        thread::sleep(Duration::from_secs(interval));

        let poll_resp = match ureq::post(&poll_url)
            .set("Accept", "application/json")
            .send_json(serde_json::json!({ "device_code": device_code }))
        {
            Ok(resp) => {
                // 202 means authorization pending (ureq treats 2xx as Ok)
                if resp.status() == 202 {
                    print!(".");
                    continue;
                }
                resp
            }
            Err(ureq::Error::Status(410, _)) => {
                bail!("Device code expired. Please try again.");
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                bail!("Poll failed (HTTP {}): {}", code, body);
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Connection error during polling: {}", t);
            }
        };

        let tokens: serde_json::Value = poll_resp
            .into_json()
            .context("Failed to parse token response")?;

        let access_token = tokens["access_token"]
            .as_str()
            .context("Missing access_token")?;
        let refresh_token = tokens["refresh_token"]
            .as_str()
            .context("Missing refresh_token")?;

        save_credentials(&Credentials {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
        })?;

        println!();
        println!("Logged in successfully.");
        return Ok(());
    }
}

fn logout() -> Result<()> {
    clear_credentials()?;
    println!("Logged out.");
    Ok(())
}

fn list() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);

    let projects = fetch_project_list(&mut client)?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!(
        "  {:<20} {:<12} {:<18} {}",
        "SLUG", "PLAN", "STATUS", "CREATED"
    );
    println!("  {}", "-".repeat(65));
    for p in &projects {
        println!(
            "  {:<20} {:<12} {:<18} {}",
            p["slug"].as_str().unwrap_or("-"),
            p["plan"].as_str().unwrap_or("-"),
            p["status"].as_str().unwrap_or("-"),
            p["created_at"].as_str().unwrap_or("-").get(..10).unwrap_or("-"),
        );
    }

    Ok(())
}

fn create(slug: Option<String>, plan: String) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);

    let slug = match slug {
        Some(s) => s,
        None => inquire::Text::new("Project slug:")
            .with_help_message("Lowercase letters, numbers, and hyphens (e.g. my-app)")
            .prompt()
            .context("Failed to read slug")?,
    };

    let resp = client.post(
        "/v1/projects",
        &serde_json::json!({ "slug": slug, "plan": plan }),
    )?;

    let project: serde_json::Value = resp.into_json().context("Failed to parse response")?;

    println!();
    println!("  Project created!");
    println!("  Slug:   {}", project["slug"].as_str().unwrap_or(&slug));
    println!("  Plan:   {}", project["plan"].as_str().unwrap_or(&plan));
    println!(
        "  Status: {}",
        project["status"].as_str().unwrap_or("pending_payment")
    );

    if is_interactive() {
        println!();
        let setup_billing = inquire::Confirm::new("Set up billing now?")
            .with_default(true)
            .prompt()
            .context("Failed to read confirmation")?;
        if setup_billing {
            wait_for_billing(&mut client, &slug)?;
            println!();
            println!("  Project is active! Run `sp00ky cloud deploy` to deploy.");
        } else {
            println!();
            println!("  Next: Run `sp00ky cloud billing` to set up payment,");
            println!("        then `sp00ky cloud deploy` to deploy.");
        }
    } else {
        println!();
        println!("  Next: Run `sp00ky cloud billing` to set up payment,");
        println!("        then `sp00ky cloud deploy` to deploy.");
    }

    Ok(())
}

/// Load environment variables from a file (KEY=val lines, skipping comments and blanks).
fn load_deploy_env_file(env_file: Option<&str>, config_dir: &std::path::Path) -> Vec<String> {
    let path = match env_file {
        Some(p) => config_dir.join(p),
        None => return Vec::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  Warning: Could not read env-file {:?}: {}", path, e);
            return Vec::new();
        }
    };
    println!("  Loaded env-file: {}", path.display());
    content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .filter(|l| l.contains('='))
        .map(|l| l.trim().to_string())
        .collect()
}

fn deploy() -> Result<()> {
    // Guided flow: ensure login → project → billing before expensive work
    let creds = ensure_login()?;
    let mut client = CloudClient::new(&creds);
    let (slug, project) = ensure_project(&mut client)?;
    let pid = project_id(&project);
    ensure_billing_active(&mut client, &slug, &project)?;

    println!();
    println!("Deploying project '{}'...", slug);

    // Load sp00ky.yml and find deployable backends
    let config_path = std::env::current_dir()?.join("sp00ky.yml");
    let config = backend::load_config(&config_path);
    config.validate()?;

    let mut backend_manifests: Vec<serde_json::Value> = Vec::new();
    let mut external_backends: Vec<serde_json::Value> = Vec::new();
    // Image hashes are stored on the API server (works cross-machine)

    for (name, backend_config) in &config.backends {
        // External backends are self-hosted — skip cloud deployment
        if backend_config.resolved_hosting() == HostingMode::External {
            println!("  Skipping external backend '{}' (self-hosted at {})", name,
                backend_config.base_url.as_deref().unwrap_or("?"));
            external_backends.push(serde_json::json!({
                "name": name,
                "base_url": backend_config.base_url,
            }));
            continue;
        }

        let deploy = match &backend_config.deploy {
            Some(d) => d,
            None => continue,
        };

        // Resolve Dockerfile path
        let dockerfile = deploy.dockerfile.clone().unwrap_or_else(|| {
            // Try to get from dev.docker config
            match &backend_config.dev {
                Some(BackendDevConfig::Typed(BackendDevTypedConfig::Docker { file, .. })) => {
                    file.clone()
                }
                _ => "Dockerfile".to_string(),
            }
        });

        let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
        let dockerfile_path = config_dir.join(&dockerfile);
        if !dockerfile_path.exists() {
            bail!(
                "Dockerfile not found for backend '{}': {}",
                name,
                dockerfile_path.display()
            );
        }

        let context_dir = match &deploy.context {
            Some(ctx) => config_dir.join(ctx),
            None => config_dir.to_path_buf(),
        };
        let image_tag = format!("sp00ky-backend-{}:deploy", name);

        // Build Docker image
        println!("  Building image for backend '{}' (dockerfile={}, context={})...",
            name, dockerfile_path.display(), context_dir.display());
        let build_status = std::process::Command::new("docker")
            .args([
                "build",
                "--platform", "linux/amd64",
                "-t",
                &image_tag,
                "-f",
                &dockerfile_path.to_string_lossy(),
                &context_dir.to_string_lossy(),
            ])
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context(format!("Failed to run docker build for backend '{}'", name))?;

        if !build_status.success() {
            bail!("Docker build failed for backend '{}'", name);
        }

        // Check if image changed since last deploy (skip export+upload if unchanged)
        let image_id = get_docker_image_id(&image_tag);
        if let Some(ref id) = image_id {
            if let Some(remote_hash) = get_remote_image_hash(&client, &pid, name) {
                if remote_hash == *id {
                    println!("  Image for backend '{}' unchanged, skipping upload.", name);
                    // Still include in manifest with previous image path
                    let resources = deploy.resources.clone().unwrap_or(backend::BackendDeployResources {
                        vcpus: 1,
                        memory: 512,
                        disk: 5,
                    });
                    backend_manifests.push(serde_json::json!({
                        "name": name,
                        "image": format!("{}/{}", slug, name),
                        "port": deploy.port,
                        "expose": deploy.expose,
                        "resources": {
                            "vcpus": resources.vcpus,
                            "memory_mb": resources.memory,
                            "disk_gb": resources.disk,
                        },
                        "env": deploy.env,
                    }));
                    println!("  Backend '{}' ready for deployment.", name);
                    continue;
                }
            }
        }

        // Export image as flat rootfs tar.gz (not layered docker save)
        let tmp_tar = std::env::temp_dir().join(format!("sp00ky-{}-{}.tar.gz", slug, name));
        println!("  Exporting image for backend '{}'...", name);
        let container_name = format!("sp00ky-export-{}-{}", slug, name);
        let _ = std::process::Command::new("docker").args(["rm", "-f", &container_name]).output();
        let create_out = std::process::Command::new("docker")
            .args(["create", "--name", &container_name, &image_tag])
            .output()
            .context(format!("Failed to create container for backend '{}'", name))?;
        if !create_out.status.success() {
            bail!("Failed to create container for backend '{}' export", name);
        }
        let export_status = std::process::Command::new("sh")
            .args(["-c", &format!("docker export {} | gzip > {}", container_name, tmp_tar.to_string_lossy())])
            .status()
            .context("Failed to export backend container")?;
        let _ = std::process::Command::new("docker").args(["rm", "-f", &container_name]).output();
        if !export_status.success() {
            bail!("Failed to export docker image for backend '{}'", name);
        }

        // Upload image tar to API with progress
        let image_data = fs::read(&tmp_tar)
            .context(format!("Failed to read image tar for backend '{}'", name))?;
        let total = image_data.len();
        println!("  Uploading image for backend '{}' ({:.1}MB)...", name, total as f64 / 1_048_576.0);

        let upload_url = format!(
            "{}/v1/projects/{}/images/{}",
            upload_base_url(&client.base_url), pid, name
        );
        let progress = ProgressReader::new(&image_data, total, &format!("  Uploading '{}'", name));
        let hash_header = image_id.as_deref().unwrap_or("");
        match ureq::put(&upload_url)
            .set("Authorization", &format!("Bearer {}", client.token))
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &total.to_string())
            .set("X-Image-Hash", hash_header)
            .send(progress)
        {
            Ok(_) => { println!(); }
            Err(ureq::Error::Status(code, resp)) => {
                println!();
                let body = resp.into_string().unwrap_or_default();
                bail!("Image upload failed for '{}' (HTTP {}): {}", name, code, body);
            }
            Err(ureq::Error::Transport(t)) => {
                println!();
                bail!("Image upload failed for '{}': {}", name, t);
            }
        }

        // Clean up temp file
        let _ = fs::remove_file(&tmp_tar);

        // Build manifest
        let resources = deploy.resources.clone().unwrap_or(backend::BackendDeployResources {
            vcpus: 1,
            memory: 512,
            disk: 5,
        });

        // Merge env-file vars with inline env (inline overrides file)
        let mut merged_env = load_deploy_env_file(deploy.env_file.as_deref(), config_dir);
        merged_env.extend(deploy.env.clone());

        backend_manifests.push(serde_json::json!({
            "name": name,
            "image": format!("{}/{}", slug, name),
            "port": deploy.port,
            "expose": deploy.expose,
            "resources": {
                "vcpus": resources.vcpus,
                "memory_mb": resources.memory,
                "disk_gb": resources.disk,
            },
            "env": merged_env,
        }));

        println!("  Backend '{}' ready for deployment.", name);
    }

    // Build SurrealDB manifest
    let resolved_surreal = config.resolved_surrealdb();
    let surrealdb_manifest = match resolved_surreal.hosting {
        HostingMode::Cloud => serde_json::json!({
            "hosting": "cloud",
            "namespace": resolved_surreal.namespace,
            "database": resolved_surreal.database,
        }),
        HostingMode::External => serde_json::json!({
            "hosting": "external",
            "endpoint": resolved_surreal.endpoint,
            "namespace": resolved_surreal.namespace,
            "database": resolved_surreal.database,
            "username": resolved_surreal.username,
            "password": resolved_surreal.password,
        }),
    };

    // Build and upload frontend if configured
    let mut frontend_manifest: Option<serde_json::Value> = None;
    if let Some(ref frontend_config) = config.frontend {
        let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
        let dockerfile_path = config_dir.join(&frontend_config.dockerfile);

        if !dockerfile_path.exists() {
            bail!("Frontend Dockerfile not found: {}", dockerfile_path.display());
        }

        let context_dir = match &frontend_config.context {
            Some(ctx) => config_dir.join(ctx),
            None => config_dir.to_path_buf(),
        };
        let image_tag = format!("sp00ky-frontend-{}:deploy", slug);

        // Auto-compute the SurrealDB WebSocket endpoint for the frontend
        // api-staging.sp00ky.cloud → db.staging.spky.cloud → wss://{slug}.db.staging.spky.cloud/rpc
        let db_ws_endpoint = derive_db_ws_endpoint(&client.base_url, &slug);

        println!("  Building frontend image...");
        let mut build_args = vec![
            "build".to_string(), "--platform".to_string(), "linux/amd64".to_string(),
            "-t".to_string(), image_tag.clone(),
            "-f".to_string(), dockerfile_path.to_string_lossy().to_string(),
        ];
        // Inject VITE_DB_ENDPOINT as build-arg so Vite picks it up during build
        if let Some(ref endpoint) = db_ws_endpoint {
            build_args.push("--build-arg".to_string());
            build_args.push(format!("VITE_DB_ENDPOINT={}", endpoint));
            println!("  DB endpoint: {}", endpoint);
        }
        build_args.push(context_dir.to_string_lossy().to_string());

        let build_status = std::process::Command::new("docker")
            .args(&build_args)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to run docker build for frontend")?;

        if !build_status.success() {
            bail!("Docker build failed for frontend");
        }

        // Check if frontend image changed since last deploy
        let frontend_image_id = get_docker_image_id(&image_tag);
        let frontend_unchanged = frontend_image_id.as_ref()
            .and_then(|id| get_remote_image_hash(&client, &pid, "frontend").map(|h| h == *id))
            .unwrap_or(false);

        if frontend_unchanged {
            println!("  Frontend image unchanged, skipping upload.");
            // Still set the manifest so the frontend gets deployed
            let resources = frontend_config.resources.clone().unwrap_or(backend::BackendDeployResources {
                vcpus: 1, memory: 512, disk: 5,
            });
            let mut merged_env = load_deploy_env_file(frontend_config.env_file.as_deref(), config_dir);
            merged_env.extend(frontend_config.env.clone());
            frontend_manifest = Some(serde_json::json!({
                "image": format!("{}/frontend", slug),
                "port": frontend_config.port,
                "resources": { "vcpus": resources.vcpus, "memory_mb": resources.memory, "disk_gb": resources.disk },
                "env": merged_env,
            }));
        } else {

        let tmp_tar = std::env::temp_dir().join(format!("sp00ky-{}-frontend.tar.gz", slug));
        println!("  Exporting frontend image...");
        // Use docker create + export to get a flat rootfs (not layered image)
        let container_name = format!("sp00ky-export-{}", slug);
        let _ = std::process::Command::new("docker").args(["rm", "-f", &container_name]).output();
        let create_status = std::process::Command::new("docker")
            .args(["create", "--name", &container_name, &image_tag])
            .output()
            .context("Failed to create container for export")?;
        if !create_status.status.success() {
            bail!("Failed to create container for frontend export");
        }
        let export_status = std::process::Command::new("sh")
            .args(["-c", &format!("docker export {} | gzip > {}", container_name, tmp_tar.to_string_lossy())])
            .status()
            .context("Failed to export frontend container")?;
        let _ = std::process::Command::new("docker").args(["rm", "-f", &container_name]).output();
        if !export_status.success() {
            bail!("Failed to export frontend container");
        }

        let image_data = fs::read(&tmp_tar).context("Failed to read frontend image tar")?;
        let total = image_data.len();
        println!("  Uploading frontend image ({:.1}MB)...", total as f64 / 1_048_576.0);
        let upload_url = format!("{}/v1/projects/{}/images/frontend", upload_base_url(&client.base_url), pid);
        let progress = ProgressReader::new(&image_data, total, "  Uploading frontend");
        let hash_header = frontend_image_id.as_deref().unwrap_or("");
        match ureq::put(&upload_url)
            .set("Authorization", &format!("Bearer {}", client.token))
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &total.to_string())
            .set("X-Image-Hash", hash_header)
            .send(progress)
        {
            Ok(_) => { println!(); }
            Err(ureq::Error::Status(code, resp)) => {
                println!();
                let body = resp.into_string().unwrap_or_default();
                bail!("Frontend image upload failed (HTTP {}): {}", code, body);
            }
            Err(ureq::Error::Transport(t)) => {
                println!();
                bail!("Frontend image upload failed: {}", t);
            }
        }
        let _ = fs::remove_file(&tmp_tar);

        let resources = frontend_config.resources.clone().unwrap_or(backend::BackendDeployResources {
            vcpus: 1,
            memory: 512,
            disk: 5,
        });

        // Merge env-file vars with inline env (inline overrides file)
        let mut merged_env = load_deploy_env_file(frontend_config.env_file.as_deref(), config_dir);
        merged_env.extend(frontend_config.env.clone());

        frontend_manifest = Some(serde_json::json!({
            "image": format!("{}/frontend", slug),
            "port": frontend_config.port,
            "resources": {
                "vcpus": resources.vcpus,
                "memory_mb": resources.memory,
                "disk_gb": resources.disk,
            },
            "env": merged_env,
        }));

        println!("  Frontend ready for deployment.");
        } // end else (frontend changed)
    }

    let ssp_count = config.deployment.as_ref()
        .and_then(|d| d.ssp_count);

    let deploy_body = serde_json::json!({
        "surrealdb": surrealdb_manifest,
        "backends": backend_manifests,
        "external_backends": external_backends,
        "frontend": frontend_manifest,
        "ssp_count": ssp_count,
    });

    let resp = client.post(
        &format!("/v1/projects/{}/deploy", pid),
        &deploy_body,
    )?;

    let deployment: serde_json::Value = resp.into_json().context("Failed to parse response")?;
    let version = deployment["version"].as_u64().unwrap_or(0);

    println!("  Deployment v{} created. Waiting for provisioning...", version);
    println!();

    // Phase 1: Stream events until infra is ready (migrating state)
    let phase1_status = stream_deployment_events(&client, &pid)?;

    match phase1_status.as_str() {
        "migrating" => {
            println!();
            println!("  ▸ Infrastructure ready. Running migrations...");

            // Get the SurrealDB URL from deployment status
            if let Ok(status_resp) = client.get(&format!("/v1/projects/{}/deployment", pid)) {
                if let Ok(status_data) = status_resp.into_json::<serde_json::Value>() {
                    if let Some(db_url) = status_data["urls"]["surrealdb"].as_str() {
                        // Wait for SurrealDB to be reachable via public URL
                        print!("  ▸ Connecting to SurrealDB");
                        let mut db_ready = false;
                        for _ in 0..30 {
                            let check = ureq::post(&format!("{}/sql", db_url))
                                .set("Accept", "application/json")
                                .send_string("INFO FOR KV;");
                            if check.is_ok() {
                                db_ready = true;
                                break;
                            }
                            print!(".");
                            thread::sleep(Duration::from_secs(2));
                        }
                        println!();

                        if !db_ready {
                            println!("  ▸ Warning: SurrealDB not reachable at {}, skipping migrations.", db_url);
                        } else {
                            let resolved = config.resolved_surrealdb();
                            // Use the auto-generated password from the deployment status
                            let db_password = status_data["surrealdb_password"]
                                .as_str()
                                .unwrap_or("");
                            let surreal_client = if db_password.is_empty() {
                                crate::surreal_client::SurrealClient::new_unauthenticated(
                                    db_url,
                                    &resolved.namespace,
                                    &resolved.database,
                                )
                            } else {
                                crate::surreal_client::SurrealClient::new(
                                    db_url,
                                    &resolved.namespace,
                                    &resolved.database,
                                    "root",
                                    db_password,
                                )
                            };

                            let schema = config.resolved_schema();
                            let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
                            let migrations_dir = config_dir.join(&schema.migrations);

                            if migrations_dir.exists() {
                                match crate::migrate::apply(&surreal_client, &migrations_dir) {
                                    Ok(()) => println!("  ▸ Migrations complete."),
                                    Err(e) => println!("  ▸ Migration warning: {:?}", e),
                                }
                            } else {
                                println!("  ▸ No migrations directory found, skipping.");
                            }

                            // Apply remote functions (DEFINE API, fn::query::register, etc.)
                            // Derive SSP endpoint from SurrealDB IP (SSP is always .30 on same subnet)
                            if let Some(db_url_str) = status_data["urls"]["surrealdb"].as_str() {
                                // Get SSP IP from deployment status, or derive from DB IP
                                let ssp_endpoint = status_data["urls"]["ssp"]
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| {
                                        // Derive from VMs list: find SSP VM IP
                                        status_data["vms"].as_array().and_then(|vms| {
                                            vms.iter()
                                                .find(|v| v["role"].as_str() == Some("ssp"))
                                                .and_then(|v| v["internal_ip"].as_str())
                                                .map(|ip| format!("http://{}:8667", ip))
                                        })
                                    });

                                // Remote functions will be applied after SSP is provisioned (phase 2)
                            }
                        }
                    }
                }
            }

            // Phase 2: Finalize deployment (triggers SSP + scheduler + app VM provisioning)
            println!("  ▸ Deploying applications...");
            client.post(
                &format!("/v1/projects/{}/deploy/finalize", pid),
                &serde_json::json!({}),
            )?;

            // Stream events until fully running
            let phase2_status = stream_deployment_events(&client, &pid)?;
            match phase2_status.as_str() {
                "running" => {
                    println!();
                    println!("  Deployment is running!");

                    // Apply remote functions now that SSP is running
                    if let Ok(status_resp) = client.get(&format!("/v1/projects/{}/deployment", pid)) {
                        if let Ok(status) = status_resp.into_json::<serde_json::Value>() {
                            print_deployment_details(&status);

                            // Find SSP endpoint and apply functions
                            if let Some(db_url) = status["urls"]["surrealdb"].as_str() {
                                let ssp_endpoint = status["urls"]["ssp"]
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| {
                                        status["vms"].as_array().and_then(|vms| {
                                            vms.iter()
                                                .find(|v| v["role"].as_str() == Some("ssp"))
                                                .and_then(|v| v["internal_ip"].as_str())
                                                .map(|ip| format!("http://{}:8667", ip))
                                        })
                                    });

                                // Use scheduler URL for remote functions (scheduler routes to SSPs)
                                // Fall back to SSP URL for singlenode mode without scheduler
                                let endpoint = status["urls"]["scheduler"]
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| ssp_endpoint.clone());

                                if let Some(fn_endpoint) = endpoint {
                                    let resolved = config.resolved_surrealdb();
                                    let db_password = status["surrealdb_password"].as_str().unwrap_or("");
                                    let surreal_client = if db_password.is_empty() {
                                        crate::surreal_client::SurrealClient::new_unauthenticated(
                                            db_url, &resolved.namespace, &resolved.database,
                                        )
                                    } else {
                                        crate::surreal_client::SurrealClient::new(
                                            db_url, &resolved.namespace, &resolved.database, "root", db_password,
                                        )
                                    };
                                    let mode = config.mode.as_deref().unwrap_or("singlenode");
                                    // Read the auth secret from the server's deployment status response.
                                    // The cloud API generates this and sets it on both scheduler and SSP VMs.
                                    let auth_secret = status["sp00ky_auth_secret"]
                                        .as_str()
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| {
                                            // Fallback: check backend env config (for local/custom setups)
                                            let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
                                            for (_name, backend_config) in &config.backends {
                                                if let Some(deploy) = &backend_config.deploy {
                                                    let mut all_env = load_deploy_env_file(deploy.env_file.as_deref(), config_dir);
                                                    all_env.extend(deploy.env.clone());
                                                    for entry in &all_env {
                                                        if let Some(val) = entry.strip_prefix("SP00KY_AUTH_SECRET=") {
                                                            return val.to_string();
                                                        }
                                                    }
                                                }
                                            }
                                            String::new()
                                        });
                                    let functions_sql = crate::schema_builder::build_remote_functions_schema(
                                        mode, &fn_endpoint, &auth_secret,
                                    );
                                    match surreal_client.execute(&functions_sql) {
                                        Ok(_) => println!("  ▸ Remote functions applied."),
                                        Err(e) => println!("  ▸ Warning: failed to apply remote functions: {:?}", e),
                                    }
                                }
                            }
                        }
                    }
                }
                "failed" => bail!("Deployment failed."),
                _ => bail!("Deployment ended unexpectedly (status: {})", phase2_status),
            }
        }
        "running" => {
            // No migration phase (legacy or no infra VMs)
            println!();
            println!("  Deployment is running!");
            if let Ok(status_resp) = client.get(&format!("/v1/projects/{}/deployment", pid)) {
                if let Ok(status) = status_resp.into_json::<serde_json::Value>() {
                    print_deployment_details(&status);
                }
            }
        }
        "failed" => bail!("Deployment failed."),
        _ => bail!("Deployment ended unexpectedly (status: {})", phase1_status),
    }

    Ok(())
}

/// Get the Docker image ID (SHA digest) for a given tag.
fn get_docker_image_id(tag: &str) -> Option<String> {
    let output = std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.Id}}", tag])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Get the remote image hash from the API.
fn get_remote_image_hash(client: &CloudClient, pid: &str, image_name: &str) -> Option<String> {
    let url = format!("{}/v1/projects/{}/images/{}/hash", client.base_url, pid, image_name);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", client.token))
        .call().ok()?;
    let data: serde_json::Value = resp.into_json().ok()?;
    let hash = data["hash"].as_str()?.to_string();
    if hash.is_empty() { None } else { Some(hash) }
}

/// A Read wrapper that prints upload progress percentage.
struct ProgressReader<'a> {
    data: &'a [u8],
    pos: usize,
    total: usize,
    label: String,
    last_pct: u8,
}

impl<'a> ProgressReader<'a> {
    fn new(data: &'a [u8], total: usize, label: &str) -> Self {
        Self { data, pos: 0, total, label: label.to_string(), last_pct: 0 }
    }
}

impl<'a> std::io::Read for ProgressReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = &self.data[self.pos..];
        let n = std::cmp::min(buf.len(), remaining.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        self.pos += n;
        let pct = if self.total > 0 { (self.pos * 100 / self.total) as u8 } else { 100 };
        if pct != self.last_pct {
            self.last_pct = pct;
            print!("\r  {} {}%", self.label, pct);
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// Deployment status display — animated inline table
// ---------------------------------------------------------------------------

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Shared state between the SSE reader and the render thread.
struct DeployState {
    phase: String,
    vms: Vec<(String, String, String)>, // (role, ip, status)
    done: bool,
}

fn vm_icon(status: &str, spinner_frame: usize) -> String {
    match status {
        "running" => " ●".to_string(),
        "starting" => format!(" {}", SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()]),
        "failed" => " ✗".to_string(),
        "stopped" => " ○".to_string(),
        _ => " ·".to_string(), // pending
    }
}

fn vm_style(status: &str) -> &str {
    match status {
        "running" => "\x1b[32m",  // green
        "starting" => "\x1b[33m", // yellow
        "failed" => "\x1b[31m",   // red
        "stopped" => "\x1b[90m",  // dim
        _ => "\x1b[90m",          // dim / pending
    }
}

/// Render the full VM table in-place by moving the cursor up and overwriting.
fn render_vm_table(
    vms: &[(String, String, String)],
    phase: &str,
    spinner_frame: usize,
    first_render: bool,
) {
    use std::io::Write;
    let mut out = std::io::stdout();

    // Layout: blank + phase + blank + header + vms + blank
    let total_lines = 4 + vms.len();

    if !first_render {
        write!(out, "\x1b[{}A", total_lines).ok();
    }

    // Phase header with padding
    let phase_label = match phase {
        "provisioning" => "Provisioning infrastructure",
        "deploying_apps" => "Deploying applications",
        "migrating" => "Running migrations",
        "running" => "Deployment complete",
        "failed" => "Deployment failed",
        _ => phase,
    };
    write!(out, "\r\x1b[2K\n").ok(); // top padding

    // Phase line with its own spinner for in-progress phases
    match phase {
        "running" => {
            write!(
                out,
                "\r\x1b[2K   \x1b[32m●\x1b[0m \x1b[1m{}\x1b[0m\n",
                phase_label
            )
            .ok();
        }
        "failed" => {
            write!(
                out,
                "\r\x1b[2K   \x1b[31m✗\x1b[0m \x1b[1m{}\x1b[0m\n",
                phase_label
            )
            .ok();
        }
        _ => {
            let f = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
            write!(
                out,
                "\r\x1b[2K   \x1b[36m{}\x1b[0m \x1b[1m{}\x1b[0m\n",
                f, phase_label
            )
            .ok();
        }
    }

    write!(out, "\r\x1b[2K\n").ok(); // padding after phase

    // Header
    write!(
        out,
        "\r\x1b[2K   \x1b[90m  {:<14} {:<18} {}\x1b[0m\n",
        "SERVICE", "IP", "STATUS"
    )
    .ok();

    // VM rows
    for (role, ip, status) in vms {
        let icon = vm_icon(status, spinner_frame);
        let style = vm_style(status);
        write!(
            out,
            "\r\x1b[2K   {}{}\x1b[0m {}{:<14}\x1b[0m \x1b[90m{:<18}\x1b[0m {}{}\x1b[0m\n",
            style, icon, style, role, ip, style, status,
        )
        .ok();
    }

    write!(out, "\r\x1b[2K").ok(); // bottom padding line (no newline, cursor stays)

    out.flush().ok();
}

/// Stream SSE deployment events and return the final status when the stream closes.
/// Uses an animated inline table with spinner for in-progress services.
fn stream_deployment_events(client: &CloudClient, pid: &str) -> Result<String> {
    use std::sync::{Arc, Mutex};

    let events_url = format!(
        "{}/v1/projects/{}/deployment/events",
        client.base_url, pid
    );
    let sse_result = ureq::get(&events_url)
        .set("Authorization", &format!("Bearer {}", client.token))
        .set("Accept", "text/event-stream")
        .call();

    match sse_result {
        Ok(sse_resp) => {
            let reader = std::io::BufReader::new(sse_resp.into_reader());
            use std::io::BufRead;

            let is_tty = std::io::stdout().is_terminal();

            let state = Arc::new(Mutex::new(DeployState {
                phase: String::from("provisioning"),
                vms: Vec::new(),
                done: false,
            }));

            // Spawn a render thread that redraws every 80ms for spinner animation
            let render_state = Arc::clone(&state);
            let render_handle = if is_tty {
                Some(thread::spawn(move || {
                    let mut frame: usize = 0;
                    let mut rendered = false;

                    loop {
                        thread::sleep(Duration::from_millis(80));

                        let s = render_state.lock().unwrap();
                        if s.done {
                            // Final render
                            render_vm_table(&s.vms, &s.phase, frame, !rendered);
                            rendered = true;
                            break;
                        }
                        if !s.vms.is_empty() {
                            render_vm_table(&s.vms, &s.phase, frame, !rendered);
                            rendered = true;
                        }
                        drop(s);

                        frame += 1;
                    }
                }))
            } else {
                None
            };

            let mut final_status = String::new();

            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let data = match line.strip_prefix("data: ") {
                    Some(d) => d,
                    None => continue,
                };
                let event: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = event["type"].as_str().unwrap_or("");
                match event_type {
                    "vm" => {
                        let role = event["role"].as_str().unwrap_or("?").to_string();
                        let ip = event["ip"].as_str().unwrap_or("?").to_string();
                        let status = event["status"].as_str().unwrap_or("?").to_string();

                        if is_tty {
                            let mut s = state.lock().unwrap();
                            if let Some(vm) = s.vms.iter_mut().find(|v| v.0 == role && v.1 == ip) {
                                vm.2 = status;
                            } else {
                                s.vms.push((role, ip, status));
                            }
                        } else {
                            let icon = match status.as_str() {
                                "running" => "●",
                                "starting" => "◐",
                                "failed" => "✗",
                                "stopped" => "○",
                                _ => "·",
                            };
                            println!("  {} {:14} {:18} {}", icon, role, ip, status);
                        }
                    }
                    "deployment" => {
                        let status = event["status"].as_str().unwrap_or("unknown");
                        final_status = status.to_string();

                        if is_tty {
                            let mut s = state.lock().unwrap();
                            s.phase = status.to_string();
                        } else {
                            match status {
                                "provisioning" => println!("  ▸ Provisioning VMs..."),
                                "deploying_apps" => println!("  ▸ Deploying applications..."),
                                "migrating" | "running" | "failed" | "destroyed" => {}
                                _ => println!("  ▸ {}", status),
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Signal render thread to stop and do final render
            if is_tty {
                {
                    let mut s = state.lock().unwrap();
                    s.done = true;
                }
                if let Some(handle) = render_handle {
                    handle.join().ok();
                }
                println!(); // final newline after table
            }

            Ok(final_status)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("Failed to stream events (HTTP {}): {}", code, body);
        }
        Err(ureq::Error::Transport(t)) => {
            bail!("Connection error: {}", t);
        }
    }
}

fn status() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (_, pid) = resolve_project_id(&mut client)?;

    let resp = client.get(&format!("/v1/projects/{}/deployment", pid))?;
    let data: serde_json::Value = resp.into_json().context("Failed to parse response")?;

    print_deployment_details(&data);
    Ok(())
}

fn logs(service: Option<String>) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (slug, pid) = resolve_project_id(&mut client)?;

    let mut path = format!("/v1/projects/{}/logs", pid);
    if let Some(ref svc) = service {
        path = format!("{}?service={}", path, svc);
    }

    println!("Streaming logs for '{}' (Ctrl+C to stop)...", slug);
    println!();

    // SSE streaming - read line by line
    let url = format!("{}{}", client.base_url, path);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", client.token))
        .set("Accept", "text/event-stream")
        .call();

    match resp {
        Ok(resp) => {
            let reader = std::io::BufReader::new(resp.into_reader());
            let reader = std::io::BufRead::lines(reader);
            for line in reader {
                match line {
                    Ok(line) => {
                        if let Some(data) = line.strip_prefix("data: ") {
                            println!("{}", data);
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("Failed to stream logs (HTTP {}): {}", code, body);
        }
        Err(ureq::Error::Transport(t)) => {
            bail!("Connection error: {}", t);
        }
    }

    Ok(())
}

fn scale(ssp: u32) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (_, pid) = resolve_project_id(&mut client)?;

    let resp = client.post(
        &format!("/v1/projects/{}/scale", pid),
        &serde_json::json!({ "ssp_count": ssp }),
    )?;

    let result: serde_json::Value = resp.into_json().context("Failed to parse response")?;
    let count = result["ssp_count"].as_u64().unwrap_or(ssp as u64);

    println!("Scaling to {} SSP instance(s)...", count);
    poll_scale_completion(&mut client, &pid, ssp)?;
    Ok(())
}

fn destroy() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (slug, pid) = resolve_project_id(&mut client)?;

    let confirmed = inquire::Confirm::new(&format!(
        "Are you sure you want to destroy project '{}'? This cannot be undone.",
        slug
    ))
    .with_default(false)
    .prompt()
    .context("Failed to read confirmation")?;

    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    client.delete(&format!("/v1/projects/{}", pid))?;

    println!("Project '{}' destroyed.", slug);
    Ok(())
}

fn backup(action: CloudBackupCommands) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (_slug, pid) = resolve_project_id(&mut client)?;

    match action {
        CloudBackupCommands::List => {
            let resp = client.get(&format!("/v1/projects/{}/backups", pid))?;
            let backups: Vec<serde_json::Value> = resp.into_json()?;
            if backups.is_empty() {
                println!("  No backups found.");
                return Ok(());
            }
            println!("  {:<36}  {:<12}  {:<10}  {}", "ID", "STATUS", "SIZE", "CREATED");
            println!("  {}", "-".repeat(80));
            for b in &backups {
                let id = b["id"].as_str().unwrap_or("?");
                let status = b["status"].as_str().unwrap_or("?");
                let size = b["size_bytes"].as_i64().unwrap_or(0);
                let created = b["created_at"].as_str().unwrap_or("?");
                let name = b["name"].as_str().unwrap_or("");
                let size_str = if size > 1_000_000 {
                    format!("{:.1} MB", size as f64 / 1_000_000.0)
                } else if size > 0 {
                    format!("{:.1} KB", size as f64 / 1_000.0)
                } else {
                    "—".to_string()
                };
                let label = if name.is_empty() { id.to_string() } else { format!("{} ({})", name, &id[..8]) };
                println!("  {:<36}  {:<12}  {:<10}  {}", label, status, size_str, &created[..19]);
            }
        }
        CloudBackupCommands::Create { name } => {
            println!("  Creating backup...");
            let body = serde_json::json!({ "name": name });
            let resp = client.post(&format!("/v1/projects/{}/backups", pid), &body)?;
            let result: serde_json::Value = resp.into_json()?;
            println!("  Backup created: {}", result["id"].as_str().unwrap_or("?"));
            println!("  Status: {}", result["status"].as_str().unwrap_or("pending"));
        }
        CloudBackupCommands::Restore { backup_id } => {
            println!("  Restoring from backup {}...", backup_id);
            let resp = client.post(
                &format!("/v1/projects/{}/backups/{}/restore", pid, backup_id),
                &serde_json::json!({}),
            )?;
            let result: serde_json::Value = resp.into_json()?;
            println!("  {}", result["message"].as_str().unwrap_or("Restore initiated"));
        }
        CloudBackupCommands::Delete { backup_id } => {
            let resp = client.delete(&format!("/v1/projects/{}/backups/{}", pid, backup_id))?;
            if resp.status() == 200 {
                println!("  Backup {} deleted.", backup_id);
            } else {
                bail!("Failed to delete backup");
            }
        }
        CloudBackupCommands::Configure { enabled, schedule, retention } => {
            let body = serde_json::json!({
                "enabled": enabled,
                "schedule": schedule,
                "retention": retention,
            });
            client.post(&format!("/v1/projects/{}/backups/configure", pid), &body)?;
            println!("  Backup configuration updated.");
        }
        CloudBackupCommands::Reset { no_backup } => {
            println!();
            println!("  ╔══════════════════════════════════════════════════════╗");
            println!("  ║  ⚠️  WARNING: DATABASE RESET                        ║");
            println!("  ║                                                      ║");
            println!("  ║  This will PERMANENTLY DELETE all data in the        ║");
            println!("  ║  database and recreate it from scratch.              ║");
            println!("  ║                                                      ║");
            println!("  ║  All user accounts, records, and application         ║");
            println!("  ║  data will be lost. Migrations will be re-applied.   ║");
            println!("  ╚══════════════════════════════════════════════════════╝");
            println!();

            let confirmed = inquire::Confirm::new(
                "Are you absolutely sure you want to reset the database?"
            )
            .with_default(false)
            .prompt()
            .context("Failed to read confirmation")?;

            if !confirmed {
                println!("  Cancelled.");
                return Ok(());
            }

            // Offer to create a backup first
            if !no_backup {
                let backup_first = inquire::Confirm::new(
                    "Create a backup before resetting?"
                )
                .with_default(true)
                .prompt()
                .context("Failed to read confirmation")?;

                if backup_first {
                    println!("  Creating backup before reset...");
                    let body = serde_json::json!({ "name": "pre-reset-backup" });
                    let resp = client.post(&format!("/v1/projects/{}/backups", pid), &body)?;
                    let result: serde_json::Value = resp.into_json()?;
                    let backup_id = result["id"].as_str().unwrap_or("?").to_string();
                    println!("  Backup {} created. Waiting for completion...", backup_id);

                    // Poll until backup completes
                    for _ in 0..60 {
                        thread::sleep(Duration::from_secs(5));
                        let status_resp = client.get(&format!("/v1/projects/{}/backups", pid))?;
                        let backups: Vec<serde_json::Value> = status_resp.into_json()?;
                        if let Some(b) = backups.iter().find(|b| b["id"].as_str() == Some(&backup_id)) {
                            match b["status"].as_str() {
                                Some("completed") => {
                                    println!("  Backup completed.");
                                    break;
                                }
                                Some("failed") => {
                                    let err = b["error"].as_str().unwrap_or("unknown");
                                    bail!("Backup failed: {}. Aborting reset.", err);
                                }
                                _ => print!("."),
                            }
                        }
                    }
                    println!();
                }
            }

            // Reset the database
            println!("  Resetting database...");
            let resp = client.post(
                &format!("/v1/projects/{}/backups/reset", pid),
                &serde_json::json!({}),
            )?;
            let result: serde_json::Value = resp.into_json()?;
            println!("  Database reset: {}", result["status"].as_str().unwrap_or("done"));

            // Re-run migrations
            println!("  Running migrations...");
            let config_path = std::env::current_dir()?.join("sp00ky.yml");
            let config = backend::load_config(&config_path);

            if let Some(db_url) = result["surrealdb_ip"].as_str() {
                let resolved = config.resolved_surrealdb();
                let db_password = result.get("db_password")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Connect directly to SurrealDB internal IP for migrations
                let surreal_url = format!("http://{}:8000", db_url);
                let surreal_client = if db_password.is_empty() {
                    crate::surreal_client::SurrealClient::new_unauthenticated(
                        &surreal_url, &resolved.namespace, &resolved.database,
                    )
                } else {
                    crate::surreal_client::SurrealClient::new(
                        &surreal_url, &resolved.namespace, &resolved.database,
                        "root", db_password,
                    )
                };

                let schema = config.resolved_schema();
                let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
                let migrations_dir = config_dir.join(&schema.migrations);

                if migrations_dir.exists() {
                    match crate::migrate::apply(&surreal_client, &migrations_dir) {
                        Ok(()) => println!("  Migrations complete."),
                        Err(e) => println!("  Migration warning: {:?}", e),
                    }
                }

                // Apply remote functions
                if let Some(sched_url) = result["scheduler_url"].as_str() {
                    let mode = config.mode.as_deref().unwrap_or("singlenode");
                    let functions_sql = crate::schema_builder::build_remote_functions_schema(
                        mode, sched_url, "",
                    );
                    match surreal_client.execute(&functions_sql) {
                        Ok(_) => println!("  Remote functions applied."),
                        Err(e) => println!("  Warning: remote functions: {:?}", e),
                    }
                }
            }

            println!();
            println!("  Database reset complete.");
        }
    }
    Ok(())
}

fn billing(action: Option<CloudBillingCommands>) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);

    match action {
        Some(CloudBillingCommands::Usage) => {
            let resp = client.get("/v1/billing/usage")?;
            let usage: Vec<serde_json::Value> =
                resp.into_json().context("Failed to parse usage")?;

            if usage.is_empty() {
                println!("No usage data for this billing period.");
                return Ok(());
            }

            println!("{:<20} {:<20} {:>10}", "PROJECT", "METRIC", "TOTAL");
            println!("{}", "-".repeat(52));
            for entry in &usage {
                println!(
                    "{:<20} {:<20} {:>10.2}",
                    entry["project"].as_str().unwrap_or("-"),
                    entry["metric"].as_str().unwrap_or("-"),
                    entry["total"].as_f64().unwrap_or(0.0),
                );
            }
            Ok(())
        }
        None => {
            let slug = resolve_project_slug()?;

            // Try portal first, fall back to checkout if no billing account yet
            match client.post("/v1/billing/portal", &serde_json::json!({})) {
                Ok(resp) => {
                    let data: serde_json::Value =
                        resp.into_json().context("Failed to parse response")?;
                    let url = data["url"]
                        .as_str()
                        .context("No billing portal URL in response")?;
                    println!("Opening billing portal...");
                    open::that(url).context("Failed to open browser")?;
                }
                Err(_) => {
                    // No billing account — start checkout and wait for payment
                    println!("No billing account found. Starting checkout for '{}'...", slug);
                    wait_for_billing(&mut client, &slug)?;
                }
            }
            Ok(())
        }
    }
}

fn upgrade(check_only: bool, target_version: Option<String>, force: bool) -> Result<()> {
    let api_base = api_base_url();

    println!("  Checking for updates...");
    println!();

    // Get latest cached versions from the control plane (public endpoint, no auth)
    let latest_resp = ureq::get(&format!("{}/v1/images/latest", api_base))
        .call()
        .context("Failed to fetch latest image versions")?;
    let latest: serde_json::Value = latest_resp.into_json().context("Failed to parse latest versions")?;

    // Try to get current running versions (requires auth — may fail gracefully)
    let mut running_versions: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut client_and_pid: Option<(CloudClient, String)> = None;

    if let Ok(creds) = require_credentials() {
        let mut client = CloudClient::new(&creds);
        if let Ok((_slug, pid)) = resolve_project_id(&mut client) {
            if let Ok(status_resp) = client.get(&format!("/v1/projects/{}/deployment", pid)) {
                if let Ok(status) = status_resp.into_json::<serde_json::Value>() {
                    // Extract versions from /api/spooky if possible
                    if let Some(db_url) = status["urls"]["surrealdb"].as_str() {
                        let config_path = std::env::current_dir().unwrap_or_default().join("sp00ky.yml");
                        let config = backend::load_config(&config_path);
                        let resolved = config.resolved_surrealdb();
                        let spooky_url = format!("{}/api/{}/{}/spooky", db_url, resolved.namespace, resolved.database);
                        if let Ok(resp) = ureq::get(&spooky_url).call() {
                            if let Ok(entities) = resp.into_json::<Vec<serde_json::Value>>() {
                                for entity in &entities {
                                    if let (Some(role), Some(version)) = (entity["entity"].as_str(), entity["version"].as_str()) {
                                        running_versions.insert(role.to_string(), version.to_string());
                                    }
                                }
                            }
                        }
                    }
                    client_and_pid = Some((client, pid.clone()));
                }
            }
        }
    }

    // running_versions was populated above (in auth block)

    // Display version comparison
    println!("  {:<14} {:<24} {:<24} {}", "ROLE", "CURRENT", "LATEST", "STATUS");
    println!("  {}", "─".repeat(70));

    let mut has_update = false;
    for role in &["scheduler", "ssp"] {
        let current = running_versions.get(*role).map(|s| s.as_str()).unwrap_or("not deployed");
        let latest_ver = latest.get(*role)
            .and_then(|v| v["version"].as_str())
            .unwrap_or("not cached");

        let status_str = if current == latest_ver {
            "up to date"
        } else if latest_ver == "not cached" {
            "no cached image"
        } else {
            has_update = true;
            "update available"
        };

        println!("  {:<14} {:<24} {:<24} {}", role, current, latest_ver, status_str);
    }
    println!();

    if check_only {
        return Ok(());
    }

    if !has_update {
        println!("  Everything is up to date.");
        return Ok(());
    }

    // Check for major version changes
    // (simplified: just check if the major version number changed)
    if !force {
        // For now, always allow canary updates without force flag
    }

    let confirmed = inquire::Confirm::new("Apply update?")
        .with_default(true)
        .prompt()
        .context("Failed to read confirmation")?;

    if !confirmed {
        println!("  Cancelled.");
        return Ok(());
    }

    println!("  Upgrading...");

    if let Some((mut client, pid)) = client_and_pid {
        // Deploy with upgrade_infra flag — tells orchestrator to replace SSP/scheduler VMs
        println!("  Redeploying with updated images...");
        let resp = client.post(
            &format!("/v1/projects/{}/deploy", pid),
            &serde_json::json!({"upgrade_infra": true}),
        )?;

        if resp.status() == 200 || resp.status() == 201 {
            println!("  Deployment triggered. Use 'sp00ky cloud status' to monitor progress.");
        } else {
            println!("  Warning: deploy returned HTTP {}", resp.status());
        }
    } else {
        bail!("Not authenticated. Run `sp00ky cloud login` first, then retry the upgrade.");
    }

    println!();
    println!("  Upgrade initiated.");
    Ok(())
}

fn keys(action: CloudKeyCommands) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);

    match action {
        CloudKeyCommands::Create => {
            let resp = client.post("/v1/auth/keys", &serde_json::json!({}))?;
            let data: serde_json::Value = resp.into_json()?;
            let key = data["key"].as_str().unwrap_or("");
            let id = data["id"].as_str().unwrap_or("");
            println!("API key created:");
            println!("  Key: {}", key);
            println!("  ID:  {}", id);
            println!();
            println!("  Save this key — it won't be shown again.");
            println!("  Use it in CI/CD: export SPOOKY_API_KEY={}", key);
        }
        CloudKeyCommands::List => {
            let resp = client.get("/v1/auth/keys")?;
            let data: Vec<serde_json::Value> = resp.into_json()?;
            if data.is_empty() {
                println!("No API keys found. Create one with `sp00ky cloud keys create`.");
            } else {
                println!("{:<38} {:<24} {}", "ID", "PREFIX", "CREATED");
                for key in &data {
                    println!(
                        "{:<38} {:<24} {}",
                        key["id"].as_str().unwrap_or("-"),
                        key["prefix"].as_str().unwrap_or("-"),
                        key["created_at"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
        CloudKeyCommands::Revoke { id } => {
            client.delete(&format!("/v1/auth/keys/{}", id))?;
            println!("API key {} revoked.", id);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------



fn print_deployment_details(data: &serde_json::Value) {
    if let Some(dep) = data.get("deployment") {
        println!(
            "  Deployment v{} - {}",
            dep["version"].as_u64().unwrap_or(0),
            dep["status"].as_str().unwrap_or("unknown")
        );
        if let Some(err) = dep["error"].as_str() {
            println!("  Error: {}", err);
        }
        println!(
            "  Created: {}",
            dep["created_at"].as_str().unwrap_or("-")
        );
    }

    if let Some(vms) = data.get("vms").and_then(|v| v.as_array()) {
        if !vms.is_empty() {
            println!();
            println!("  {:<12} {:<16} {:<10}", "ROLE", "IP", "STATUS");
            println!("  {}", "-".repeat(40));
            for vm in vms {
                println!(
                    "  {:<12} {:<16} {:<10}",
                    vm["role"].as_str().unwrap_or("-"),
                    vm["internal_ip"].as_str().unwrap_or("-"),
                    vm["status"].as_str().unwrap_or("-"),
                );
            }
        }
    }

    if let Some(urls) = data.get("urls").and_then(|v| v.as_object()) {
        if !urls.is_empty() {
            println!();
            println!("  Endpoints:");
            for (name, url) in urls {
                println!("    {} {}", name, url.as_str().unwrap_or("-"));
            }
        }

        // Show component versions from /api/spooky
        if let Some(db_url) = urls.get("surrealdb").and_then(|v| v.as_str()) {
            let config_path = std::env::current_dir().unwrap_or_default().join("sp00ky.yml");
            let config = backend::load_config(&config_path);
            let resolved = config.resolved_surrealdb();
            let spooky_url = format!("{}/api/{}/{}/spooky", db_url, resolved.namespace, resolved.database);
            if let Ok(resp) = ureq::get(&spooky_url).timeout(std::time::Duration::from_secs(5)).call() {
                if let Ok(entities) = resp.into_json::<Vec<serde_json::Value>>() {
                    if !entities.is_empty() {
                        println!();
                        println!("  {:<14} {:<24} {}", "COMPONENT", "VERSION", "STATUS");
                        println!("  {}", "-".repeat(50));
                        for entity in &entities {
                            println!(
                                "  {:<14} {:<24} {}",
                                entity["entity"].as_str().unwrap_or("-"),
                                entity["version"].as_str().unwrap_or("-"),
                                entity["status"].as_str().unwrap_or("-"),
                            );
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Link (GitHub auto-deploy)
// ---------------------------------------------------------------------------

fn link(action: CloudLinkCommands) -> Result<()> {
    match action {
        CloudLinkCommands::Setup => link_setup(),
        CloudLinkCommands::Status => link_status(),
        CloudLinkCommands::Settings { branch, auto_deploy } => link_settings(branch, auto_deploy),
        CloudLinkCommands::Unlink => link_unlink(),
        CloudLinkCommands::Trigger => link_trigger(),
        CloudLinkCommands::Runs => link_runs(),
    }
}

fn link_setup() -> Result<()> {
    let creds = ensure_login()?;
    let mut client = CloudClient::new(&creds);
    let (_slug, pid) = resolve_project_id(&mut client)?;

    // Initiate setup
    let resp = client.post(
        &format!("/v1/projects/{}/link/setup", pid),
        &serde_json::json!({}),
    )?;
    let data: serde_json::Value = resp.into_json()?;
    let install_url = data["install_url"]
        .as_str()
        .context("Missing install_url in response")?;

    println!("Opening GitHub to install the Sp00ky Cloud app...");
    println!("  {}", install_url);
    let _ = open::that(install_url);

    // Poll until linked
    println!();
    println!("Waiting for GitHub App installation...");

    let mut attempts = 0;
    let max_attempts = 120; // 6 minutes at 3s intervals

    loop {
        thread::sleep(Duration::from_secs(3));
        attempts += 1;

        if attempts > max_attempts {
            bail!("Timed out waiting for GitHub App installation. Try again with `sp00ky cloud link setup`.");
        }

        let resp = client.get(&format!("/v1/projects/{}/link", pid))?;
        let data: serde_json::Value = resp.into_json()?;
        let status = data["status"].as_str().unwrap_or("");

        match status {
            "linked" => {
                let repo_owner = data["repo_owner"].as_str().unwrap_or("");
                let repo_name = data["repo_name"].as_str().unwrap_or("");
                let branch = data["branch"].as_str().unwrap_or("main");

                println!("Linked to {}/{} on branch '{}'", repo_owner, repo_name, branch);
                println!("  Auto-deploy: {}", if data["auto_deploy"].as_bool().unwrap_or(true) { "enabled" } else { "disabled" });

                // Offer to deploy now
                if is_interactive() {
                    let deploy_now = inquire::Confirm::new("Deploy now?")
                        .with_default(true)
                        .prompt()
                        .unwrap_or(false);

                    if deploy_now {
                        let resp = client.post(
                            &format!("/v1/projects/{}/link/trigger", pid),
                            &serde_json::json!({}),
                        )?;
                        let run: serde_json::Value = resp.into_json()?;
                        println!("Build triggered (run: {})", run["run_id"].as_str().unwrap_or("?"));
                        println!("  Commit: {}", run["commit_sha"].as_str().unwrap_or("?"));
                    }
                }
                return Ok(());
            }
            "pending_repo_selection" => {
                // Installation complete but multiple repos — user needs to pick one
                if let Some(repos) = data["repos"].as_array() {
                    if repos.is_empty() {
                        continue;
                    }

                    let repo_names: Vec<String> = repos
                        .iter()
                        .map(|r| r["full_name"].as_str().unwrap_or("?").to_string())
                        .collect();

                    println!();
                    let selection = inquire::Select::new("Select repository:", repo_names)
                        .prompt()
                        .context("Failed to select repository")?;

                    // Parse owner/name
                    let parts: Vec<&str> = selection.split('/').collect();
                    if parts.len() != 2 {
                        bail!("Invalid repository format");
                    }

                    client.patch(
                        &format!("/v1/projects/{}/link", pid),
                        &serde_json::json!({
                            "repo_owner": parts[0],
                            "repo_name": parts[1],
                        }),
                    )?;

                    println!("Linked to {}", selection);

                    // Offer to deploy
                    if is_interactive() {
                        let deploy_now = inquire::Confirm::new("Deploy now?")
                            .with_default(true)
                            .prompt()
                            .unwrap_or(false);

                        if deploy_now {
                            let resp = client.post(
                                &format!("/v1/projects/{}/link/trigger", pid),
                                &serde_json::json!({}),
                            )?;
                            let run: serde_json::Value = resp.into_json()?;
                            println!("Build triggered (run: {})", run["run_id"].as_str().unwrap_or("?"));
                        }
                    }
                    return Ok(());
                }
            }
            "not_linked" => {
                // Still waiting
                if attempts % 10 == 0 {
                    print!(".");
                }
            }
            _ => {}
        }
    }
}

fn link_status() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (slug, pid) = resolve_project_id(&mut client)?;

    let resp = client.get(&format!("/v1/projects/{}/link", pid))?;
    let data: serde_json::Value = resp.into_json()?;
    let status = data["status"].as_str().unwrap_or("unknown");

    match status {
        "linked" => {
            println!("Project: {}", slug);
            println!("Repository: {}/{}",
                data["repo_owner"].as_str().unwrap_or("?"),
                data["repo_name"].as_str().unwrap_or("?"));
            println!("Branch: {}", data["branch"].as_str().unwrap_or("?"));
            println!("Auto-deploy: {}", if data["auto_deploy"].as_bool().unwrap_or(true) { "enabled" } else { "disabled" });

            if let Some(runs) = data["runs"].as_array() {
                if !runs.is_empty() {
                    println!();
                    println!("Recent runs:");
                    println!("  {:<10} {:<12} {:<20} {}", "STATUS", "COMMIT", "TIME", "MESSAGE");
                    println!("  {}", "-".repeat(60));
                    for run in runs {
                        let sha = run["commit_sha"].as_str().unwrap_or("?");
                        let short_sha = if sha.len() > 8 { &sha[..8] } else { sha };
                        println!("  {:<10} {:<12} {:<20} {}",
                            run["status"].as_str().unwrap_or("?"),
                            short_sha,
                            run["created_at"].as_str().unwrap_or("?").get(..19).unwrap_or("?"),
                            run["commit_message"].as_str().unwrap_or("").lines().next().unwrap_or(""),
                        );
                    }
                }
            }
        }
        "not_linked" => {
            println!("Project '{}' is not linked to a GitHub repository.", slug);
            println!("Run `sp00ky cloud link setup` to set up automated deployments.");
        }
        _ => {
            println!("Link status: {}", status);
        }
    }

    Ok(())
}

fn link_settings(branch: Option<String>, auto_deploy: Option<bool>) -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (_slug, pid) = resolve_project_id(&mut client)?;

    // If no flags provided, use interactive prompts
    let (branch, auto_deploy) = if branch.is_none() && auto_deploy.is_none() && is_interactive() {
        // Get current settings first
        let resp = client.get(&format!("/v1/projects/{}/link", pid))?;
        let data: serde_json::Value = resp.into_json()?;
        if data["status"].as_str() != Some("linked") {
            bail!("Project is not linked. Run `sp00ky cloud link setup` first.");
        }

        let current_branch = data["branch"].as_str().unwrap_or("main").to_string();
        let current_auto = data["auto_deploy"].as_bool().unwrap_or(true);

        let new_branch = inquire::Text::new("Branch:")
            .with_default(&current_branch)
            .prompt()
            .context("Failed to read branch")?;

        let new_auto = inquire::Confirm::new("Auto-deploy on push?")
            .with_default(current_auto)
            .prompt()
            .context("Failed to read auto-deploy setting")?;

        (Some(new_branch), Some(new_auto))
    } else {
        (branch, auto_deploy)
    };

    let mut body = serde_json::Map::new();
    if let Some(b) = &branch {
        body.insert("branch".to_string(), serde_json::Value::String(b.clone()));
    }
    if let Some(a) = auto_deploy {
        body.insert("auto_deploy".to_string(), serde_json::Value::Bool(a));
    }

    client.patch(
        &format!("/v1/projects/{}/link", pid),
        &serde_json::Value::Object(body),
    )?;

    println!("Settings updated.");
    if let Some(b) = &branch {
        println!("  Branch: {}", b);
    }
    if let Some(a) = auto_deploy {
        println!("  Auto-deploy: {}", if a { "enabled" } else { "disabled" });
    }

    Ok(())
}

fn link_unlink() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (slug, pid) = resolve_project_id(&mut client)?;

    if is_interactive() {
        let confirm = inquire::Confirm::new(&format!("Unlink project '{}'? This will stop automated deployments.", slug))
            .with_default(false)
            .prompt()
            .context("Failed to read confirmation")?;

        if !confirm {
            println!("Cancelled.");
            return Ok(());
        }
    }

    client.delete(&format!("/v1/projects/{}/link", pid))?;
    println!("Project '{}' unlinked from GitHub.", slug);

    Ok(())
}

fn link_trigger() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (slug, pid) = resolve_project_id(&mut client)?;

    let resp = client.post(
        &format!("/v1/projects/{}/link/trigger", pid),
        &serde_json::json!({}),
    )?;
    let data: serde_json::Value = resp.into_json()?;

    println!("Build triggered for '{}'", slug);
    println!("  Run ID: {}", data["run_id"].as_str().unwrap_or("?"));
    println!("  Commit: {}", data["commit_sha"].as_str().unwrap_or("?"));
    println!("  Status: {}", data["status"].as_str().unwrap_or("pending"));

    Ok(())
}

fn link_runs() -> Result<()> {
    let creds = require_credentials()?;
    let mut client = CloudClient::new(&creds);
    let (_slug, pid) = resolve_project_id(&mut client)?;

    let resp = client.get(&format!("/v1/projects/{}/link/runs", pid))?;
    let runs: Vec<serde_json::Value> = resp.into_json()?;

    if runs.is_empty() {
        println!("No build runs found. Push to the linked branch or run `sp00ky cloud link trigger`.");
        return Ok(());
    }

    println!("{:<10} {:<12} {:<20} {}", "STATUS", "COMMIT", "TIME", "MESSAGE");
    println!("{}", "-".repeat(70));

    for run in &runs {
        let sha = run["commit_sha"].as_str().unwrap_or("?");
        let short_sha = if sha.len() > 8 { &sha[..8] } else { sha };
        let time = run["created_at"].as_str().unwrap_or("?");
        let short_time = time.get(..19).unwrap_or(time);
        let msg = run["commit_message"].as_str().unwrap_or("")
            .lines().next().unwrap_or("");
        let msg_truncated = if msg.len() > 40 { &msg[..37] } else { msg };

        println!("{:<10} {:<12} {:<20} {}",
            run["status"].as_str().unwrap_or("?"),
            short_sha,
            short_time,
            msg_truncated,
        );
    }

    Ok(())
}
