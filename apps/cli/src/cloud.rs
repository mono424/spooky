use std::fs;
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
}

impl CloudClient {
    fn new(token: &str) -> Self {
        Self {
            base_url: api_base_url(),
            token: token.to_string(),
        }
    }

    fn get(&self, path: &str) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
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
                let body = resp.into_string().unwrap_or_default();
                bail!("API error (HTTP {}): {}", code, body)
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }

    fn post(&self, path: &str, body: &serde_json::Value) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
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
                let body = resp.into_string().unwrap_or_default();
                bail!("API error (HTTP {}): {}", code, body)
            }
            Err(ureq::Error::Transport(t)) => {
                bail!("Failed to connect to Sp00ky Cloud API: {}", t)
            }
        }
    }

    fn delete(&self, path: &str) -> Result<ureq::Response> {
        let url = format!("{}{}", self.base_url, path);
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
                let body = resp.into_string().unwrap_or_default();
                bail!("API error (HTTP {}): {}", code, body)
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

    // 2. Current directory name as fallback
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("sp00ky.yml").exists() {
            if let Some(name) = cwd.file_name() {
                return Ok(name.to_string_lossy().to_string());
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
// Command dispatch
// ---------------------------------------------------------------------------

pub fn run(action: CloudCommands) -> Result<()> {
    match action {
        CloudCommands::Login => login(),
        CloudCommands::Logout => logout(),
        CloudCommands::Create { slug, plan } => create(slug, plan),
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
            let body = resp.into_string().unwrap_or_default();
            bail!("Login failed (HTTP {}): {}", code, body);
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

    println!();
    println!("  Your verification code is: {}", user_code);
    println!();
    println!("  Opening browser to: {}", verification_url);
    println!("  (If the browser doesn't open, visit the URL manually)");
    println!();

    // Open browser
    let _ = open::that(verification_url);

    // Poll for completion
    let poll_url = format!("{}/v1/auth/login/poll", base_url);

    loop {
        thread::sleep(Duration::from_secs(interval));

        let poll_resp = match ureq::post(&poll_url)
            .set("Accept", "application/json")
            .send_json(serde_json::json!({ "device_code": device_code }))
        {
            Ok(resp) => resp,
            Err(ureq::Error::Status(202, _)) => {
                // Authorization pending
                print!(".");
                continue;
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

fn create(slug: Option<String>, plan: String) -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);

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
    println!("  Status: {}", project["status"].as_str().unwrap_or("pending_payment"));
    println!();
    println!("  Next: Run `sp00ky cloud billing` to set up payment,");
    println!("        then `sp00ky cloud deploy` to deploy.");

    Ok(())
}

fn deploy() -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);
    let slug = resolve_project_slug()?;

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
            client.base_url, slug, name
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
        &format!("/v1/projects/{}/deploy", slug),
        &deploy_body,
    )?;

    let deployment: serde_json::Value = resp.into_json().context("Failed to parse response")?;
    let version = deployment["version"].as_u64().unwrap_or(0);

    println!("  Deployment v{} created. Waiting for provisioning...", version);
    println!();

    // Poll for status
    loop {
        thread::sleep(Duration::from_secs(2));

        let status_resp = client.get(&format!("/v1/projects/{}/deployment", slug))?;
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

fn status() -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);
    let slug = resolve_project_slug()?;

    let resp = client.get(&format!("/v1/projects/{}/deployment", slug))?;
    let data: serde_json::Value = resp.into_json().context("Failed to parse response")?;

    print_deployment_details(&data);
    Ok(())
}

fn logs(service: Option<String>) -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);
    let slug = resolve_project_slug()?;

    let mut path = format!("/v1/projects/{}/logs", slug);
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
    let client = CloudClient::new(&creds.access_token);
    let slug = resolve_project_slug()?;

    let resp = client.post(
        &format!("/v1/projects/{}/scale", slug),
        &serde_json::json!({ "ssp_count": ssp }),
    )?;

    let result: serde_json::Value = resp.into_json().context("Failed to parse response")?;
    let count = result["ssp_count"].as_u64().unwrap_or(ssp as u64);

    println!("Scaling to {} SSP instance(s). This may take a moment.", count);
    Ok(())
}

fn destroy() -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);
    let slug = resolve_project_slug()?;

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

    client.delete(&format!("/v1/projects/{}", slug))?;

    println!("Project '{}' destroyed.", slug);
    Ok(())
}

fn billing(action: Option<CloudBillingCommands>) -> Result<()> {
    let creds = require_credentials()?;
    let client = CloudClient::new(&creds.access_token);

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
            // Open billing portal in browser
            let resp = client.post("/v1/billing/portal", &serde_json::json!({}))?;
            let data: serde_json::Value =
                resp.into_json().context("Failed to parse response")?;

            let url = data["url"]
                .as_str()
                .context("No billing portal URL in response")?;

            println!("Opening billing portal...");
            open::that(url).context("Failed to open browser")?;
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
