use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::backend::{self, BackendDevConfig, BackendDevTypedConfig, HostingMode};
use crate::{CloudBillingCommands, CloudCommands};

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
}

impl CloudClient {
    fn new(creds: &Credentials) -> Self {
        Self {
            base_url: api_base_url(),
            token: creds.access_token.clone(),
            refresh_token: creds.refresh_token.clone(),
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
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::get(&url)
                    .set("Authorization", &format!("Bearer {}", self.token))
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
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/json")
            .send_json(body.clone())
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::post(&url)
                    .set("Authorization", &format!("Bearer {}", self.token))
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
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(401, _)) => {
                self.try_refresh().map_err(|_| {
                    anyhow::anyhow!("Session expired. Run `sp00ky cloud login` to re-authenticate.")
                })?;
                match ureq::delete(&url)
                    .set("Authorization", &format!("Bearer {}", self.token))
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
        CloudCommands::Billing { action } => billing(action),
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

        let context_dir = dockerfile_path.parent().unwrap_or(config_dir);
        let image_tag = format!("sp00ky-backend-{}:deploy", name);

        // Build Docker image
        println!("  Building image for backend '{}'...", name);
        let build_status = std::process::Command::new("docker")
            .args([
                "build",
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

        // Save image as tar.gz
        let tmp_tar = std::env::temp_dir().join(format!("sp00ky-{}-{}.tar.gz", slug, name));
        println!("  Saving image for backend '{}'...", name);
        let save_status = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "docker save {} | gzip > {}",
                    image_tag,
                    tmp_tar.to_string_lossy()
                ),
            ])
            .status()
            .context("Failed to save docker image")?;

        if !save_status.success() {
            bail!("Failed to save docker image for backend '{}'", name);
        }

        // Upload image tar to API
        println!("  Uploading image for backend '{}'...", name);
        let image_data = fs::read(&tmp_tar)
            .context(format!("Failed to read image tar for backend '{}'", name))?;

        let upload_url = format!(
            "{}/v1/projects/{}/images/{}",
            client.base_url, pid, name
        );
        match ureq::put(&upload_url)
            .set("Authorization", &format!("Bearer {}", client.token))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&image_data)
        {
            Ok(_) => {}
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                bail!("Image upload failed for '{}' (HTTP {}): {}", name, code, body);
            }
            Err(ureq::Error::Transport(t)) => {
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
        }),
    };

    let deploy_body = serde_json::json!({
        "surrealdb": surrealdb_manifest,
        "backends": backend_manifests,
        "external_backends": external_backends,
    });

    let resp = client.post(
        &format!("/v1/projects/{}/deploy", pid),
        &deploy_body,
    )?;

    let deployment: serde_json::Value = resp.into_json().context("Failed to parse response")?;
    let version = deployment["version"].as_u64().unwrap_or(0);

    println!("  Deployment v{} created. Waiting for provisioning...", version);
    println!();

    // Stream deployment events via SSE
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

            let mut final_status = String::new();
            let mut final_error = String::new();

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
                        let role = event["role"].as_str().unwrap_or("?");
                        let ip = event["ip"].as_str().unwrap_or("?");
                        let status = event["status"].as_str().unwrap_or("?");
                        let icon = match status {
                            "starting" => "◐",
                            "running" => "●",
                            "failed" => "✗",
                            "stopped" => "○",
                            _ => "·",
                        };
                        println!("  {} {:12} {:16} {}", icon, role, ip, status);
                    }
                    "deployment" => {
                        let status = event["status"].as_str().unwrap_or("unknown");
                        final_status = status.to_string();
                        if let Some(err) = event["error"].as_str() {
                            if !err.is_empty() {
                                final_error = err.to_string();
                            }
                        }
                        match status {
                            "provisioning" => {
                                println!("  ▸ Provisioning VMs...");
                            }
                            "running" | "failed" | "destroyed" => {
                                // Stream will close, handled below
                            }
                            _ => {
                                println!("  ▸ {}", status);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Stream ended — check final state
            match final_status.as_str() {
                "running" => {
                    println!();
                    println!("  Deployment is running!");
                    // Fetch final details
                    if let Ok(status_resp) = client.get(&format!("/v1/projects/{}/deployment", pid)) {
                        if let Ok(status) = status_resp.into_json::<serde_json::Value>() {
                            print_deployment_details(&status);
                        }
                    }
                }
                "failed" => {
                    bail!("Deployment failed: {}", if final_error.is_empty() { "unknown error" } else { &final_error });
                }
                "destroyed" => {
                    bail!("Deployment was destroyed.");
                }
                _ => {
                    bail!("Deployment stream ended unexpectedly (status: {})", final_status);
                }
            }
        }
        Err(_) => {
            // Fallback: poll for status (server may not support SSE yet)
            loop {
                thread::sleep(Duration::from_secs(2));

                let status_resp = client.get(&format!("/v1/projects/{}/deployment", pid))?;
                let status: serde_json::Value =
                    status_resp.into_json().context("Failed to parse status")?;

                let dep_status = status["deployment"]["status"]
                    .as_str()
                    .unwrap_or("unknown");

                match dep_status {
                    "running" => {
                        println!("  Deployment is running!");
                        print_deployment_details(&status);
                        return Ok(());
                    }
                    "failed" => {
                        let error = status["deployment"]["error"]
                            .as_str()
                            .unwrap_or("unknown error");
                        bail!("Deployment failed: {}", error);
                    }
                    "destroyed" => {
                        bail!("Deployment was destroyed.");
                    }
                    _ => {
                        print!("  Status: {}...\r", dep_status);
                    }
                }
            }
        }
    }

    Ok(())
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
}
