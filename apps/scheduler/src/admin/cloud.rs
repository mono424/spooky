//! The link to Sp00ky Cloud.
//!
//! Some things an operator wants from the dashboard are not the scheduler's
//! to do: pulling a newer image, wiping its own volume (RocksDB holds the
//! files open), bouncing SurrealDB, and the backup catalog with its schedule
//! and retention. Those belong to the control plane, which owns the
//! containers and the backup rows. This module is how the scheduler asks it.
//!
//! Configuration is two non-secret env vars the control plane injects,
//! `SPKY_CLOUD_API_URL` and `SPKY_CLOUD_PROJECT`, plus the `SPKY_AUTH_SECRET`
//! the scheduler already holds as its cluster identity. The control plane
//! accepts that secret on a small, allow-listed internal route family for the
//! project it belongs to. No new credential is minted or stored anywhere.
//!
//! A scheduler without the link (a checkout, a self-hosted stack) is not
//! broken, it is simply unlinked: every cloud-only action answers 409 with
//! the same sentence, which the dashboard shows beside the disabled option.

use axum::http::StatusCode;
use serde_json::{json, Value};
use tracing::warn;

use super::{api_error, ApiError};

pub const NOT_LINKED: &str = "Not linked to Sp00ky Cloud";

#[derive(Clone)]
pub struct CloudLink {
    pub api_url: String,
    pub project: String,
    secret: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for CloudLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudLink")
            .field("api_url", &self.api_url)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

impl CloudLink {
    /// `Some` only when all three inputs are present and non-empty.
    pub fn from_env() -> Option<Self> {
        let api_url = std::env::var("SPKY_CLOUD_API_URL").ok()?;
        let project = std::env::var("SPKY_CLOUD_PROJECT").ok()?;
        let secret = std::env::var("SPKY_AUTH_SECRET").ok()?;
        Self::new(api_url, project, secret)
    }

    pub fn new(api_url: String, project: String, secret: String) -> Option<Self> {
        let api_url = api_url.trim().trim_end_matches('/').to_string();
        let project = project.trim().to_string();
        if api_url.is_empty() || project.is_empty() || secret.is_empty() {
            return None;
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Self {
            api_url,
            project,
            secret,
            client,
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v1/internal/projects/{}{}",
            self.api_url, self.project, path
        )
    }

    /// Forward a request and relay the control plane's verdict.
    ///
    /// Its error bodies are `{error, code}`; the admin plane's are `{error}`.
    /// The status is passed through untouched because it is meaningful to the
    /// dashboard (a 400 `invalid_schedule` and a 409 lease conflict both need
    /// to reach the operator as what they are).
    pub async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value), ApiError> {
        let url = self.url(path);
        let mut req = self
            .client
            .request(method.clone(), &url)
            .bearer_auth(&self.secret);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.map_err(|e| {
            warn!(error = %e, %url, "Control plane unreachable");
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Sp00ky Cloud unreachable: {e}"),
            )
        })?;
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let text = res.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        if status.is_success() {
            Ok((status, value))
        } else {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Sp00ky Cloud answered {}", status.as_u16()));
            warn!(%method, %url, %status, %message, "Control plane refused");
            Err(api_error(status, message))
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.call(reqwest::Method::GET, path, None)
            .await
            .map(|(_, v)| v)
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<(StatusCode, Value), ApiError> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, ApiError> {
        self.call(reqwest::Method::DELETE, path, None)
            .await
            .map(|(_, v)| v)
    }
}

pub fn not_linked() -> ApiError {
    api_error(StatusCode::CONFLICT, NOT_LINKED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_needs_all_three_inputs() {
        assert!(CloudLink::new("https://api".into(), "slug".into(), "s".into()).is_some());
        assert!(CloudLink::new("".into(), "slug".into(), "s".into()).is_none());
        assert!(CloudLink::new("https://api".into(), " ".into(), "s".into()).is_none());
        assert!(CloudLink::new("https://api".into(), "slug".into(), "".into()).is_none());
    }

    #[test]
    fn urls_are_project_scoped_and_slash_safe() {
        let link = CloudLink::new("https://api.example/".into(), "wp".into(), "s".into()).unwrap();
        assert_eq!(
            link.url("/backups"),
            "https://api.example/v1/internal/projects/wp/backups"
        );
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let link = CloudLink::new("https://api".into(), "wp".into(), "hunter2".into()).unwrap();
        assert!(!format!("{link:?}").contains("hunter2"));
    }
}
