use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Connection settings for the main SurrealDB.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DbConfig {
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

/// Split a DB URL into (address, secure). Accepts ws://, wss://, http://,
/// https:// and bare host:port so legacy `SPKY_DB_WS` values keep working.
pub fn normalize_url(url: &str) -> (&str, bool) {
    if let Some(rest) = url.strip_prefix("wss://") {
        (rest, true)
    } else if let Some(rest) = url.strip_prefix("ws://") {
        (rest, false)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (rest, true)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (rest, false)
    } else {
        (url, false)
    }
}

/// Open a fresh HTTP connection to the main SurrealDB and sign in as root,
/// WITHOUT selecting a namespace/database. Callers that need to DEFINE the
/// namespace/database before selecting it (self-heal on a brand-new SurrealDB)
/// use this and select afterwards.
///
/// We use the HTTP engine (not WS) because `Surreal::import()` / `Surreal::export()`
/// are only implemented for HTTP and local storage engines. Calling `.import()` on
/// a WebSocket client returns `BackupsNotSupported`.
pub async fn connect_http_raw(
    db_config: &DbConfig,
) -> Result<surrealdb::Surreal<surrealdb::engine::remote::http::Client>> {
    let (addr, secure) = normalize_url(&db_config.url);

    let db = if secure {
        surrealdb::Surreal::new::<surrealdb::engine::remote::http::Https>(addr)
            .await
            .with_context(|| format!("Failed to open HTTPS to {}", db_config.url))?
    } else {
        surrealdb::Surreal::new::<surrealdb::engine::remote::http::Http>(addr)
            .await
            .with_context(|| format!("Failed to open HTTP to {}", db_config.url))?
    };

    db.signin(surrealdb::opt::auth::Root {
        username: db_config.username.clone(),
        password: db_config.password.clone(),
    })
    .await
    .context("Remote SurrealDB signin failed")?;

    Ok(db)
}

/// Open a fresh HTTP connection to the main SurrealDB: root signin plus
/// namespace/database selection.
pub async fn connect_http(
    db_config: &DbConfig,
) -> Result<surrealdb::Surreal<surrealdb::engine::remote::http::Client>> {
    let db = connect_http_raw(db_config).await?;

    db.use_ns(&db_config.namespace)
        .use_db(&db_config.database)
        .await
        .context("Failed to select remote namespace/database")?;

    Ok(db)
}

/// Keep a long-lived HTTP-engine handle authenticated.
///
/// Unlike the WS engine (one signin per connection, session lives as long as
/// the socket), the HTTP engine attaches the signin token to every request —
/// and root tokens have a finite duration. A handle held for hours (SSP shared
/// connection, scheduler feature-flag sweep) would start failing with auth
/// errors once the token lapses. Re-signing in on an interval well below the
/// token lifetime keeps the stored token fresh; a failed attempt is retried on
/// the next tick.
pub fn spawn_periodic_resignin(
    db: surrealdb::Surreal<surrealdb::engine::remote::http::Client>,
    db_config: DbConfig,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(1)));
        interval.tick().await; // skip the immediate first tick — caller just signed in
        loop {
            interval.tick().await;
            resignin_once(&db, &db_config).await;
        }
    });
}

/// One re-signin attempt (the timer-driven flavor of
/// [`spawn_periodic_resignin`] — the standalone SSP fires this from its
/// `DbResignin` wakeup). Failures are logged; the next wakeup retries.
pub async fn resignin_once(
    db: &surrealdb::Surreal<surrealdb::engine::remote::http::Client>,
    db_config: &DbConfig,
) {
    if let Err(e) = db
        .signin(surrealdb::opt::auth::Root {
            username: db_config.username.clone(),
            password: db_config.password.clone(),
        })
        .await
    {
        tracing::warn!(error = %e, "Periodic SurrealDB re-signin failed; retrying next tick");
    }
}

/// Default cadence for [`spawn_periodic_resignin`] — well below the shortest
/// common token duration (1h).
pub const RESIGNIN_INTERVAL_SECS: u64 = 900;

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn scheme_normalization() {
        assert_eq!(normalize_url("ws://host:8000"), ("host:8000", false));
        assert_eq!(normalize_url("wss://host:8000"), ("host:8000", true));
        assert_eq!(normalize_url("http://host:8000"), ("host:8000", false));
        assert_eq!(normalize_url("https://host:8000"), ("host:8000", true));
        assert_eq!(normalize_url("host:8000"), ("host:8000", false));
    }
}
