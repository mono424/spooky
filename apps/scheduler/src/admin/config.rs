//! Admin-plane configuration, read from the environment.
//!
//! Everything here is opt-out rather than opt-in: the dashboard is part of what
//! a scheduler is, so it comes up by default. What is *not* on by default is
//! the break-glass password — an unset `SPKY_ADMIN_PASSWORD` disables that
//! login path entirely rather than falling back to some default secret.

/// Default port for the admin listener.
///
/// Deliberately NOT the ingest port. `/ingest`, `/proxy/query` (arbitrary
/// SurrealQL against the replica), `/ssp/*` and `PUT /backends` are all
/// unauthenticated by design, on the assumption that 9667 is reachable only
/// from inside the deployment's private network. The dashboard is the first
/// thing here meant to be reached by a human browser, so it gets its own
/// listener that can be published without publishing any of that.
pub const DEFAULT_ADMIN_PORT: u16 = 9668;

/// Where the built dashboard lives in the scheduler image.
pub const DEFAULT_ADMIN_DIR: &str = "/usr/share/spooky/dashboard";

#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// Directory holding the built dashboard (`index.html` + assets).
    pub dir: std::path::PathBuf,
    /// Break-glass password. `None` disables password-only login.
    pub password: Option<String>,
    /// Pins the SurrealDB record-access method used for admin signin. When
    /// `None`, the accesses are discovered from `INFO FOR DB`.
    pub access: Option<String>,
    /// How long a signed-in session lasts.
    pub session_ttl: std::time::Duration,
}

impl AdminConfig {
    pub fn from_env() -> Self {
        let enabled = !matches!(
            std::env::var("SPKY_ADMIN_ENABLED").as_deref(),
            Ok("0") | Ok("false") | Ok("off")
        );

        let port = std::env::var("SPKY_ADMIN_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p > 0)
            .unwrap_or(DEFAULT_ADMIN_PORT);

        let session_ttl_secs = std::env::var("SPKY_ADMIN_SESSION_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(8 * 60 * 60);

        Self {
            enabled,
            host: std::env::var("SPKY_ADMIN_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port,
            dir: std::env::var("SPKY_ADMIN_DIR")
                .unwrap_or_else(|_| DEFAULT_ADMIN_DIR.to_string())
                .into(),
            // An empty value is treated as unset: a deployment that templates
            // this var but leaves it blank must not end up with a password of
            // "" that anyone can guess.
            password: std::env::var("SPKY_ADMIN_PASSWORD")
                .ok()
                .filter(|s| !s.is_empty()),
            access: std::env::var("SPKY_ADMIN_ACCESS")
                .ok()
                .filter(|s| !s.is_empty()),
            session_ttl: std::time::Duration::from_secs(session_ttl_secs),
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
