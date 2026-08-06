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

/// The concrete HTTP-engine handle every long-lived caller talks to.
pub type HttpDb = surrealdb::Surreal<surrealdb::engine::remote::http::Client>;

/// True for errors that mean "this handle's session is gone; a new signin on it
/// cannot help".
///
/// The HTTP engine tags each request with the UUID of a session that lives in
/// SurrealDB's *memory*. Once the server forgets that UUID the only fix is a new
/// handle, so these are the errors [`ReconnectingDb`] reconnects on rather than
/// retries:
///
/// * `Session not found: <uuid>` — server restarted (its session map is empty)
///   or the session was detached.
/// * `The session has expired` — the session outlived its auth duration.
/// * `401 Unauthorized` — what the RPC endpoint returns for a request whose
///   session can no longer be authenticated.
pub fn is_dead_session_error(msg: &str) -> bool {
    msg.contains("Session not found")
        || msg.contains("session has expired")
        || msg.contains("401 Unauthorized")
}

/// A long-lived SurrealDB handle that survives a SurrealDB restart.
///
/// # Why this exists
///
/// The HTTP engine is only *nominally* stateless. On connect it sends an RPC
/// `attach`, which registers a session in a `HashMap<Uuid, Session>` held in the
/// SurrealDB **process memory**, and every later request carries that UUID. When
/// SurrealDB restarts, that map is empty again, so every request from a
/// previously-connected handle fails with `Session not found: <uuid>` — forever.
///
/// Re-running `signin` on the same handle cannot recover it: the signin is
/// itself routed through the dead session UUID and fails the same way. That is
/// exactly what a plain `spawn_periodic_resignin` loop used to do, so a single
/// SurrealDB restart left the SSP and scheduler permanently unable to reach the
/// database — no job drain, no view registration, no realtime — until their own
/// containers were restarted.
///
/// `ReconnectingDb` closes that hole: the periodic tick probes the current
/// handle and, when the probe says the session is dead, builds a *brand-new*
/// handle (which attaches a fresh session) and swaps it in atomically. Callers
/// hold the `ReconnectingDb`, not the raw handle, so they pick up the
/// replacement on their next call without being restarted.
pub struct ReconnectingDb {
    /// Only ever held long enough to clone the `Arc`; never across an await.
    current: std::sync::RwLock<std::sync::Arc<HttpDb>>,
    config: DbConfig,
    /// Poked by [`ReconnectingDb::note_error`] so a dead session is replaced on
    /// the next data-path failure instead of waiting out the probe interval.
    wake: tokio::sync::Notify,
}

impl ReconnectingDb {
    /// Wrap an already-connected handle.
    pub fn new(db: HttpDb, config: DbConfig) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            current: std::sync::RwLock::new(std::sync::Arc::new(db)),
            config,
            wake: tokio::sync::Notify::new(),
        })
    }

    /// Connect (signin + ns/db select) and wrap the result.
    pub async fn connect(config: &DbConfig) -> Result<std::sync::Arc<Self>> {
        Ok(Self::new(connect_http(config).await?, config.clone()))
    }

    /// The handle to use right now.
    ///
    /// Cloning the returned `Arc` is free. Cloning the `Surreal` inside it is
    /// NOT — `Surreal::clone` mints a new session id and attaches it server-side
    /// — so callers must go through the `Arc` and never clone the inner handle.
    pub fn handle(&self) -> std::sync::Arc<HttpDb> {
        self.current
            .read()
            .expect("ReconnectingDb lock poisoned")
            .clone()
    }

    /// Report an error seen on the data path. A dead-session error wakes the
    /// refresh task immediately; anything else is ignored (transient transport
    /// failures recover on their own and must not churn the connection).
    pub fn note_error(&self, msg: &str) {
        if is_dead_session_error(msg) {
            self.wake.notify_one();
        }
    }

    /// One maintenance pass. Returns `true` if the handle is usable afterwards.
    ///
    /// Ordering matters: `signin` doubles as the liveness probe *and* the token
    /// refresh, so a healthy handle costs exactly one request per tick. Only
    /// when it fails do we pay for a reconnect.
    pub async fn refresh(&self) -> bool {
        let db = self.handle();
        let err = match db
            .signin(surrealdb::opt::auth::Root {
                username: self.config.username.clone(),
                password: self.config.password.clone(),
            })
            .await
        {
            Ok(_) => return true,
            Err(e) => e.to_string(),
        };

        if !is_dead_session_error(&err) {
            // SurrealDB is unreachable or erroring for some other reason. A new
            // connection would fail the same way, and the existing session may
            // still be perfectly valid once the blip passes — leave it alone.
            tracing::warn!(error = %err, "SurrealDB re-signin failed; retrying next tick");
            return false;
        }

        tracing::warn!(
            error = %err,
            "SurrealDB session is gone (server restarted?); reconnecting"
        );

        // Always reconnect via `connect_http`, even for handles originally
        // opened with `connect_http_raw`: by the time a reconnect is needed the
        // namespace/database exist (the raw handle's caller defined them), and
        // the replacement has to come back with them selected.
        match connect_http(&self.config).await {
            Ok(fresh) => {
                *self
                    .current
                    .write()
                    .expect("ReconnectingDb lock poisoned") = std::sync::Arc::new(fresh);
                tracing::info!("Reconnected to SurrealDB with a fresh session");
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "SurrealDB reconnect failed; retrying next tick");
                false
            }
        }
    }
}

/// Keep a long-lived handle usable: refresh its auth token and replace it
/// outright if its server-side session dies.
///
/// Ticks on `interval_secs`, and early whenever [`ReconnectingDb::note_error`]
/// sees a dead-session error on the data path — so the common case (SurrealDB
/// restarted under us) heals on the next failed query rather than at the next
/// tick.
pub fn spawn_periodic_resignin(db: std::sync::Arc<ReconnectingDb>, interval_secs: u64) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(1)));
        interval.tick().await; // skip the immediate first tick — caller just signed in
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = db.wake.notified() => {}
            }
            db.refresh().await;
        }
    });
}

/// React to dead-session reports from the data path, with no timer of its own.
///
/// For hosts that already drive the periodic refresh through their own
/// scheduler (the SSP arms `TimerKind::DbResignin`, so that a Durable-Object
/// shell can fire it from `alarm()`) and therefore do not run
/// [`spawn_periodic_resignin`]. Without this, [`ReconnectingDb::note_error`]
/// would have no listener and a SurrealDB restart would wait out the full
/// refresh interval instead of healing on the first failed query.
pub fn spawn_dead_session_healer(db: std::sync::Arc<ReconnectingDb>) {
    tokio::spawn(async move {
        loop {
            db.wake.notified().await;
            db.refresh().await;
        }
    });
}

/// One refresh attempt (the timer-driven flavor of
/// [`spawn_periodic_resignin`] — the standalone SSP fires this from its
/// `DbResignin` wakeup). Failures are logged; the next wakeup retries.
pub async fn resignin_once(db: &ReconnectingDb) {
    db.refresh().await;
}

/// Default cadence for [`spawn_periodic_resignin`].
///
/// This is both the token-refresh cadence (well below the shortest common token
/// duration of 1h) and the worst-case detection window for a SurrealDB restart
/// that no data-path error has reported yet — hence a minute rather than the
/// token lifetime.
pub const RESIGNIN_INTERVAL_SECS: u64 = 60;

#[cfg(test)]
mod tests {
    use super::normalize_url;

    use super::is_dead_session_error;

    /// The three strings below are copied verbatim out of a production incident
    /// where SurrealDB restarted at 19:58 and the SSP + scheduler stayed broken
    /// until their containers were restarted 84 minutes later. Every one of them
    /// has to route to "reconnect", not "retry the same handle".
    #[test]
    fn dead_session_errors_are_recognized() {
        assert!(is_dead_session_error(
            "transport: Session not found: bf4e163e-c09f-42c5-a589-9bb8421d917b"
        ));
        assert!(is_dead_session_error(
            "transport: HTTP status client error (401 Unauthorized) for url (http://surrealdb:8000/rpc)"
        ));
        assert!(is_dead_session_error("The session has expired"));
    }

    /// A SurrealDB that is merely unreachable must NOT trigger a reconnect: the
    /// existing session is still valid on the other side of the blip, and a new
    /// connection would fail identically anyway.
    #[test]
    fn transient_transport_errors_are_not_dead_sessions() {
        assert!(!is_dead_session_error(
            "transport: error sending request for url (http://surrealdb:8000/rpc)"
        ));
        assert!(!is_dead_session_error(
            "Failed to open HTTP to http://surrealdb:8000"
        ));
        assert!(!is_dead_session_error("Failed to query _00_feature_flag"));
    }

    #[test]
    fn scheme_normalization() {
        assert_eq!(normalize_url("ws://host:8000"), ("host:8000", false));
        assert_eq!(normalize_url("wss://host:8000"), ("host:8000", true));
        assert_eq!(normalize_url("http://host:8000"), ("host:8000", false));
        assert_eq!(normalize_url("https://host:8000"), ("host:8000", true));
        assert_eq!(normalize_url("host:8000"), ("host:8000", false));
    }
}
