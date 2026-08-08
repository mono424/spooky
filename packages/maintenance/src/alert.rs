//! Outbound alerting for health probes: a dead-man's-switch ping plus an
//! edge-triggered failure/recovery webhook.
//!
//! Two channels, deliberately complementary:
//!
//! - **Ping URL** (`SPKY_HEARTBEAT_PING_URL`): GET on every successful probe
//!   cycle. Point it at a heartbeat monitor (e.g. Betterstack) that alerts
//!   when pings STOP. This is the primary channel — a wedged process cannot
//!   report its own wedge, but it also cannot keep pinging, so absence is the
//!   alert. Covers the failure modes the process can't detect.
//! - **Webhook URL** (`SPKY_ALERT_WEBHOOK_URL`): POST a JSON payload on the
//!   ok→failed edge (after `fail_threshold` consecutive failures, so one
//!   flaky cycle never pages) and on the failed→ok edge (`recovered`). Covers
//!   the failure modes the process CAN detect, faster than ping absence.
//!
//! Every send is spawned fire-and-forget: alerting must never block or slow
//! the probe loop it reports on.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, warn};

pub struct Alerter {
    client: reqwest::Client,
    ping_url: Option<String>,
    webhook_url: Option<String>,
    fail_threshold: u32,
    /// Edge detector: true while we consider the probe failed (i.e. we have
    /// sent a `failed` webhook and not yet a `recovered` one).
    currently_failed: AtomicBool,
}

impl Alerter {
    pub fn new(
        ping_url: Option<String>,
        webhook_url: Option<String>,
        fail_threshold: u32,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build alert HTTP client"),
            ping_url: ping_url.filter(|s| !s.is_empty()),
            webhook_url: webhook_url.filter(|s| !s.is_empty()),
            fail_threshold: fail_threshold.max(1),
            currently_failed: AtomicBool::new(false),
        }
    }

    /// Report one probe cycle. `payload` is only invoked when a webhook will
    /// actually fire (edge transitions), so building it can be as expensive
    /// as needed. Never blocks: all sends are spawned.
    pub fn observe(
        &self,
        ok: bool,
        consecutive_failures: u32,
        payload: impl FnOnce() -> serde_json::Value,
    ) {
        if ok {
            if let Some(url) = &self.ping_url {
                let client = self.client.clone();
                let url = url.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.get(&url).send().await {
                        debug!(error = %e, "heartbeat ping failed to send");
                    }
                });
            }
            // failed → ok edge
            if self.currently_failed.swap(false, Ordering::SeqCst) {
                self.send_webhook(serde_json::json!({
                    "status": "recovered",
                    "detail": payload(),
                }));
            }
        } else if consecutive_failures >= self.fail_threshold
            && !self.currently_failed.swap(true, Ordering::SeqCst)
        {
            // ok → failed edge, debounced by the threshold
            self.send_webhook(serde_json::json!({
                "status": "failed",
                "consecutive_failures": consecutive_failures,
                "detail": payload(),
            }));
        }
    }

    fn send_webhook(&self, body: serde_json::Value) {
        let Some(url) = &self.webhook_url else { return };
        let client = self.client.clone();
        let url = url.clone();
        tokio::spawn(async move {
            match client.post(&url).json(&body).send().await {
                Ok(resp) if !resp.status().is_success() => {
                    warn!(status = %resp.status(), "alert webhook returned non-success");
                }
                Ok(_) => {}
                Err(e) => warn!(error = %e, "alert webhook failed to send"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn drain_spawned() {
        // Give the fire-and-forget send tasks a chance to run.
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn pings_on_every_ok_cycle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ping"))
            .respond_with(ResponseTemplate::new(200))
            .expect(3)
            .mount(&server)
            .await;

        let alerter = Alerter::new(Some(format!("{}/ping", server.uri())), None, 3);
        for _ in 0..3 {
            alerter.observe(true, 0, || serde_json::json!({}));
        }
        drain_spawned().await;
    }

    #[tokio::test]
    async fn webhook_fires_only_on_edges_and_respects_threshold() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(body_partial_json(serde_json::json!({"status": "failed"})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(body_partial_json(serde_json::json!({"status": "recovered"})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let alerter = Alerter::new(None, Some(format!("{}/hook", server.uri())), 3);

        // Below threshold: nothing fires.
        alerter.observe(false, 1, || serde_json::json!({}));
        alerter.observe(false, 2, || serde_json::json!({}));
        // Threshold crossed: exactly one `failed`, even across repeats.
        alerter.observe(false, 3, || serde_json::json!({}));
        alerter.observe(false, 4, || serde_json::json!({}));
        alerter.observe(false, 5, || serde_json::json!({}));
        // Recovery: exactly one `recovered`, repeats stay quiet.
        alerter.observe(true, 0, || serde_json::json!({}));
        alerter.observe(true, 0, || serde_json::json!({}));

        drain_spawned().await;
    }
}
