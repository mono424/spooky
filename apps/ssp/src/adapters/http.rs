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

        // Headers AND body inside one future, so both halves of the exchange are
        // covered by the cancel race and by the error mapping below.
        //
        // The body read used to sit after the `select!`, with its error discarded by
        // `unwrap_or_default()`. Two things went wrong there: a kill fired while the
        // body was streaming was never observed, and — worse — a response whose
        // headers arrived 2xx and whose body then stalled past the deadline was
        // reported as a SUCCESSFUL empty-body response. A job that never delivered its
        // output was recorded as having succeeded with no output, which is the one
        // outcome an operator cannot tell from a real empty result.
        let exchange = async {
            let resp = builder.send().await?;
            let status = resp.status().as_u16();
            let body = resp.text().await?;
            Ok::<_, reqwest::Error>(OutboundResponse { status, body })
        };

        let response = if let Some(mut cancel) = cancel {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(HttpError::Cancelled),
                resp = exchange => resp,
            }
        } else {
            exchange.await
        };

        match response {
            Ok(resp) => Ok(resp),
            Err(e) if e.is_timeout() => Err(HttpError::Timeout(req.timeout)),
            Err(e) => Err(HttpError::Transport(e.to_string())),
        }
    }
}
