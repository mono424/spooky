use ssp_node::{CancelWatch, HttpClient, HttpError, Method, OutboundRequest, OutboundResponse};

/// `ssp_node::HttpClient` over reqwest, with cancel-wins race semantics
/// (mirrors the job runner's `select! { biased; cancel, send }` kill path).
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

impl Default for ReqwestHttp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl HttpClient for ReqwestHttp {
    async fn send(
        &self,
        req: OutboundRequest,
        cancel: Option<CancelWatch>,
    ) -> Result<OutboundResponse, HttpError> {
        let mut builder = match req.method {
            Method::Get => self.client.get(&req.url),
            Method::Post => self.client.post(&req.url),
            Method::Put => self.client.put(&req.url),
            Method::Delete => self.client.delete(&req.url),
        }
        .timeout(req.timeout);
        if let Some(bearer) = &req.bearer {
            builder = builder.bearer_auth(bearer);
        }
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &req.json_body {
            builder = builder.json(body);
        }

        let send_fut = builder.send();

        let response = if let Some(mut cancel) = cancel {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(HttpError::Cancelled),
                resp = send_fut => resp,
            }
        } else {
            send_fut.await
        };

        match response {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Ok(OutboundResponse { status, body })
            }
            Err(e) if e.is_timeout() => Err(HttpError::Timeout(req.timeout)),
            Err(e) => Err(HttpError::Transport(e.to_string())),
        }
    }
}
