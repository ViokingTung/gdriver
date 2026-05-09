//! Authenticated HTTP client for Google Drive REST API v3.
//!
//! [`DriveClient`] wraps [`reqwest::Client`] with automatic Bearer-token
//! injection, 401-triggered token refresh (one attempt), and exponential
//! backoff for 429 / 5xx responses (up to 3 retries).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::redirect;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

// ─── TokenRefresher ─────────────────────────────────────────────────────────

/// A trait for providing a fresh access token when the current one expires.
///
/// The daemon wires this to its OAuth config + OS keychain so [`DriveClient`]
/// can self-heal after a 401 without the caller managing token lifetimes.
#[async_trait]
pub trait TokenRefresher: Send + Sync {
    /// Return a new access token (e.g. by calling the Google token endpoint
    /// with a stored refresh token).
    async fn refresh(&self) -> anyhow::Result<String>;
}

// ─── DriveClient ────────────────────────────────────────────────────────────

/// An authenticated HTTP client for a single Google account.
///
/// All public methods take `&self` — the access token is stored behind a
/// [`RwLock`] so concurrent API calls are safe and token updates (via
/// [`set_access_token`](Self::set_access_token) or the internal 401-refresh
/// path) are visible to every inflight request.
pub struct DriveClient {
    http: reqwest::Client,
    /// A second client with redirect-following disabled, used for resumable
    /// upload chunk PUTs so that Google's 308 "Resume Incomplete" response
    /// is returned to the caller instead of being treated as a redirect.
    http_no_redirect: reqwest::Client,
    access_token: RwLock<String>,
    token_refresher: Option<Arc<dyn TokenRefresher>>,
}

impl DriveClient {
    /// Create a new client that authenticates with `access_token`.
    pub fn new(access_token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            http_no_redirect: reqwest::Client::builder()
                .redirect(redirect::Policy::none())
                .build()
                .expect("failed to build no-redirect HTTP client"),
            access_token: RwLock::new(access_token.into()),
            token_refresher: None,
        }
    }

    /// Attach a [`TokenRefresher`] so the client can automatically recover
    /// from 401 responses by obtaining a fresh access token.
    pub fn with_refresher(mut self, refresher: Arc<dyn TokenRefresher>) -> Self {
        self.token_refresher = Some(refresher);
        self
    }

    /// Replace the cached access token (e.g. after an external refresh).
    pub fn set_access_token(&self, token: impl Into<String>) {
        let mut t = self.access_token.write().unwrap_or_else(|e| e.into_inner());
        *t = token.into();
    }

    /// Return a clone of the current access token.
    pub fn access_token(&self) -> String {
        self.access_token.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    // ── Public HTTP helpers ────────────────────────────────────────────────

    /// `GET {url}` → deserialised JSON body.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let req = self.http.get(url);
        self.execute_json::<T>(req, url, "GET").await
    }

    /// `POST {url}` with JSON request body → deserialised JSON response.
    pub async fn post_json<T: DeserializeOwned>(
        &self,
        url: &str,
        body: &impl serde::Serialize,
    ) -> anyhow::Result<T> {
        let req = self.http.post(url).json(body);
        self.execute_json::<T>(req, url, "POST").await
    }

    /// `PATCH {url}` with JSON request body → deserialised JSON response.
    pub async fn patch_json<T: DeserializeOwned>(
        &self,
        url: &str,
        body: &impl serde::Serialize,
    ) -> anyhow::Result<T> {
        let req = self.http.patch(url).json(body);
        self.execute_json::<T>(req, url, "PATCH").await
    }

    /// `DELETE {url}` → deserialised JSON body.
    pub async fn delete_json<T: DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let req = self.http.delete(url);
        self.execute_json::<T>(req, url, "DELETE").await
    }

    /// `DELETE {url}` → success / failure (no response body expected).
    ///
    /// Use this for endpoints that return 204 No Content.
    pub async fn delete_empty(&self, url: &str) -> anyhow::Result<()> {
        let req = self.http.delete(url);
        self.execute_no_content(req, url, "DELETE").await
    }

    /// `GET {url}` → raw [`reqwest::Response`] for streaming.
    ///
    /// The caller is responsible for reading the response body.  Retry / token
    /// refresh logic still applies — non-2xx responses are consumed for error
    /// reporting and the request is retried.
    pub async fn get_raw(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        let req = self.http.get(url);
        self.execute_raw(req, url, "GET").await
    }

    /// `POST {url}` with raw body and custom `Content-Type`.
    ///
    /// Returns the raw [`reqwest::Response`] so the caller can inspect status
    /// codes and headers (needed by resumable upload for the `Location` header).
    /// Retry / token refresh logic still applies.
    pub async fn post_raw(
        &self,
        url: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let req = self
            .http
            .post(url)
            .header(CONTENT_TYPE, content_type)
            .body(body);
        self.execute_raw(req, url, "POST").await
    }

    /// `PUT {url}` with raw body, extra headers, and **no redirect following**.
    ///
    /// Google's resumable upload protocol uses HTTP 308 as a "Resume Incomplete"
    /// signal — reqwest would normally follow 308 redirects, which loses the
    /// status code and `Range` header.  This method uses a client built with
    /// [`redirect::Policy::none`] so the caller receives the raw 308 response.
    ///
    /// Still retries on 401 / 429 / 5xx.
    pub async fn put_raw_no_redirect(
        &self,
        url: &str,
        body: Vec<u8>,
        extra_headers: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut req = self.http_no_redirect.put(url);
        for (k, v) in extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.body(body);
        self.execute_raw_no_redirect(req, url, "PUT").await
    }

    // ── Core retry / refresh engine ────────────────────────────────────────

    /// Send the request, deserialise on success, retry on transient failures.
    ///
    /// Retry policy:
    /// - **401** — call [`TokenRefresher::refresh`] once, then retry.
    /// - **429 / 5xx** — exponential backoff: 2s, 4s, 8s (max 3 attempts).
    /// - Other 4xx — immediate error.
    async fn execute_json<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
        url: &str,
        method: &str,
    ) -> anyhow::Result<T> {
        let mut attempt: u32 = 0;
        let max_retries: u32 = 3;
        let mut token_refreshed = false;

        loop {
            let token = self.access_token();

            // Clone the request builder so we can reuse it across attempts.
            let req = builder
                .try_clone()
                .context("failed to clone request — body may not be replayable")?;

            let resp = req
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .header(CONTENT_TYPE, "application/json")
                .send()
                .await
                .context("Drive API request failed")?;

            let status = resp.status();
            debug!("{method} {url} → HTTP {status}");

            if status.is_success() {
                return resp
                    .json()
                    .await
                    .context("failed to deserialise Drive API response");
            }

            // ── 401: try token refresh once ────────────────────────────
            if status.as_u16() == 401 && !token_refreshed {
                if let Some(refresher) = &self.token_refresher {
                    warn!("{method} {url} → 401, attempting token refresh");
                    match refresher.refresh().await {
                        Ok(new_token) => {
                            debug!("token refreshed successfully (len={})", new_token.len());
                            self.set_access_token(new_token);
                            token_refreshed = true;
                            continue;
                        }
                        Err(e) => {
                            warn!("token refresh failed: {e:#}");
                        }
                    }
                }
            }

            // ── 429 / 5xx: exponential backoff ────────────────────────
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < max_retries {
                attempt += 1;
                let delay = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "{method} {url} → HTTP {status}, retry {attempt}/{max_retries} after {delay:?}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            // ── Terminal ─────────────────────────────────────────────
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Drive API error (HTTP {status} on {method} {url}): {body:.500}"
            ));
        }
    }

    /// Like [`execute_json`] but for endpoints that return 204 No Content.
    async fn execute_no_content(
        &self,
        builder: reqwest::RequestBuilder,
        url: &str,
        method: &str,
    ) -> anyhow::Result<()> {
        let mut attempt: u32 = 0;
        let max_retries: u32 = 3;
        let mut token_refreshed = false;

        loop {
            let token = self.access_token();
            let req = builder
                .try_clone()
                .context("failed to clone request")?;

            let resp = req
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
                .context("Drive API request failed")?;

            let status = resp.status();
            debug!("{method} {url} → HTTP {status}");

            if status.is_success() {
                return Ok(());
            }

            if status.as_u16() == 401 && !token_refreshed {
                if let Some(refresher) = &self.token_refresher {
                    warn!("{method} {url} → 401, attempting token refresh");
                    match refresher.refresh().await {
                        Ok(new_token) => {
                            debug!("token refreshed successfully (len={})", new_token.len());
                            self.set_access_token(new_token);
                            token_refreshed = true;
                            continue;
                        }
                        Err(e) => warn!("token refresh failed: {e:#}"),
                    }
                }
            }

            if (status.as_u16() == 429 || status.is_server_error()) && attempt < max_retries {
                attempt += 1;
                let delay = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "{method} {url} → HTTP {status}, retry {attempt}/{max_retries} after {delay:?}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Drive API error (HTTP {status} on {method} {url}): {body:.500}"
            ));
        }
    }

    /// Like [`execute_json`] but returns the raw response for streaming.
    async fn execute_raw(
        &self,
        builder: reqwest::RequestBuilder,
        url: &str,
        method: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let mut attempt: u32 = 0;
        let max_retries: u32 = 3;
        let mut token_refreshed = false;

        loop {
            let token = self.access_token();
            let req = builder
                .try_clone()
                .context("failed to clone request")?;

            let resp = req
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
                .context("Drive API request failed")?;

            let status = resp.status();
            debug!("{method} {url} → HTTP {status}");

            if status.is_success() {
                return Ok(resp);
            }

            // Consume error body before retry.
            let _body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 401 && !token_refreshed {
                if let Some(refresher) = &self.token_refresher {
                    warn!("{method} {url} → 401, attempting token refresh");
                    match refresher.refresh().await {
                        Ok(new_token) => {
                            debug!("token refreshed successfully (len={})", new_token.len());
                            self.set_access_token(new_token);
                            token_refreshed = true;
                            continue;
                        }
                        Err(e) => warn!("token refresh failed: {e:#}"),
                    }
                }
            }

            if (status.as_u16() == 429 || status.is_server_error()) && attempt < max_retries {
                attempt += 1;
                let delay = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "{method} {url} → HTTP {status}, retry {attempt}/{max_retries} after {delay:?}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            return Err(anyhow::anyhow!(
                "Drive API error (HTTP {status} on {method} {url}): {_body:.500}"
            ));
        }
    }

    /// Like [`execute_raw`] but treats HTTP 308 as an acceptable status.
    ///
    /// Google's resumable upload protocol uses 308 (Resume Incomplete) as a
    /// business-level signal, not a redirect.  The caller inspects the status
    /// and headers to decide the next action.  401 / 429 / 5xx are still
    /// retried per the standard policy.
    async fn execute_raw_no_redirect(
        &self,
        builder: reqwest::RequestBuilder,
        url: &str,
        method: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let mut attempt: u32 = 0;
        let max_retries: u32 = 3;
        let mut token_refreshed = false;

        loop {
            let token = self.access_token();
            let req = builder
                .try_clone()
                .context("failed to clone request")?;

            let resp = req
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
                .context("Drive API request failed")?;

            let status = resp.status();
            debug!("{method} {url} → HTTP {status}");

            if status.is_success() || status.as_u16() == 308 {
                return Ok(resp);
            }

            // Consume error body before retry.
            let _body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 401 && !token_refreshed {
                if let Some(refresher) = &self.token_refresher {
                    warn!("{method} {url} → 401, attempting token refresh");
                    match refresher.refresh().await {
                        Ok(new_token) => {
                            debug!("token refreshed successfully (len={})", new_token.len());
                            self.set_access_token(new_token);
                            token_refreshed = true;
                            continue;
                        }
                        Err(e) => warn!("token refresh failed: {e:#}"),
                    }
                }
            }

            if (status.as_u16() == 429 || status.is_server_error()) && attempt < max_retries {
                attempt += 1;
                let delay = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "{method} {url} → HTTP {status}, retry {attempt}/{max_retries} after {delay:?}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            return Err(anyhow::anyhow!(
                "Drive API error (HTTP {status} on {method} {url}): {_body:.500}"
            ));
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::sync::Mutex;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Simple JSON payload used across tests.
    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Ping {
        message: String,
    }

    // ── helpers ──────────────────────────────────────────────────────────

    fn test_client(token: &str) -> DriveClient {
        DriveClient::new(token)
    }

    /// A [`TokenRefresher`] that returns a fixed token and records call count.
    struct CountingRefresher {
        token: String,
        call_count: Mutex<u32>,
    }

    impl CountingRefresher {
        fn new(token: &str) -> Self {
            Self { token: token.into(), call_count: Mutex::new(0) }
        }

        fn call_count(&self) -> u32 {
            *self.call_count.lock().unwrap()
        }
    }

    #[async_trait]
    impl TokenRefresher for CountingRefresher {
        async fn refresh(&self) -> anyhow::Result<String> {
            *self.call_count.lock().unwrap() += 1;
            Ok(self.token.clone())
        }
    }

    // ── Basic auth header injection ──────────────────────────────────────

    #[tokio::test]
    async fn injects_bearer_token() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test"))
            .and(header("Authorization", "Bearer test-token-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Ping {
                message: "ok".into(),
            }))
            .mount(&server)
            .await;

        let client = test_client("test-token-123");
        let resp: Ping = client.get_json(&format!("{}/test", server.uri())).await.unwrap();
        assert_eq!(resp, Ping { message: "ok".into() });
    }

    #[tokio::test]
    async fn post_json_sends_and_deserialises() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/ct"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Ping {
                message: "ct".into(),
            }))
            .mount(&server)
            .await;

        let client = test_client("tok");
        let body = serde_json::json!({"key": "val"});
        let resp: Ping = client
            .post_json(&format!("{}/ct", server.uri()), &body)
            .await
            .unwrap();
        assert_eq!(resp.message, "ct");
    }

    // ── 401 → token refresh → retry succeeds ────────────────────────────

    #[tokio::test]
    async fn refresh_on_401_then_retry_succeeds() {
        let server = MockServer::start().await;
        let url = format!("{}/needs-fresh-token", server.uri());

        let refresher = Arc::new(CountingRefresher::new("fresh-token"));

        // First call: 401 with the old token
        Mock::given(method("GET"))
            .and(path("/needs-fresh-token"))
            .and(header("Authorization", "Bearer expired-token"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        // Second call: 200 with the fresh token
        Mock::given(method("GET"))
            .and(path("/needs-fresh-token"))
            .and(header("Authorization", "Bearer fresh-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Ping {
                message: "recovered".into(),
            }))
            .mount(&server)
            .await;

        let client = test_client("expired-token").with_refresher(refresher.clone());
        let resp: Ping = client.get_json(&url).await.unwrap();
        assert_eq!(resp, Ping { message: "recovered".into() });
        assert_eq!(refresher.call_count(), 1);
    }

    #[tokio::test]
    async fn refresh_called_only_once_even_on_consecutive_401s() {
        let server = MockServer::start().await;
        let url = format!("{}/double-401", server.uri());

        let refresher = Arc::new(CountingRefresher::new("still-bad-token"));

        // Both calls return 401 (fresh token is also rejected by this mock).
        Mock::given(method("GET"))
            .and(path("/double-401"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = test_client("expired").with_refresher(refresher.clone());
        let result: anyhow::Result<Ping> = client.get_json(&url).await;

        assert!(result.is_err(), "should fail after refresh attempt exhausted");
        assert_eq!(refresher.call_count(), 1, "refresh called exactly once");
    }

    // ── 429 / 5xx exponential backoff ────────────────────────────────────

    #[tokio::test]
    async fn retries_on_429_with_backoff() {
        let server = MockServer::start().await;
        let url = format!("{}/rate-limited", server.uri());

        let call_count = std::sync::atomic::AtomicU32::new(0);

        Mock::given(method("GET"))
            .and(path("/rate-limited"))
            .respond_with(move |_req: &wiremock::Request| {
                let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    // First two calls → 429
                    ResponseTemplate::new(429)
                } else {
                    // Third call → 200
                    ResponseTemplate::new(200).set_body_raw(r#"{"message":"finally"}"#, "application/json")
                }
            })
            .mount(&server)
            .await;

        let client = test_client("tok");
        let resp: Ping = client.get_json(&url).await.unwrap();
        assert_eq!(resp, Ping { message: "finally".into() });
    }

    #[tokio::test]
    async fn retries_on_5xx_with_backoff() {
        let server = MockServer::start().await;
        let url = format!("{}/server-error", server.uri());

        let call_count = std::sync::atomic::AtomicU32::new(0);

        Mock::given(method("GET"))
            .and(path("/server-error"))
            .respond_with(move |_req: &wiremock::Request| {
                let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    ResponseTemplate::new(503)
                } else {
                    ResponseTemplate::new(200).set_body_raw(r#"{"message":"recovered"}"#, "application/json")
                }
            })
            .mount(&server)
            .await;

        let client = test_client("tok");
        let resp: Ping = client.get_json(&url).await.unwrap();
        assert_eq!(resp.message, "recovered");
    }

    #[tokio::test]
    async fn gives_up_after_max_retries() {
        let server = MockServer::start().await;
        let url = format!("{}/always-down", server.uri());

        // 4 identical 503 responses (initial + 3 retries = 4 attempts total)
        Mock::given(method("GET"))
            .and(path("/always-down"))
            .respond_with(ResponseTemplate::new(503))
            .expect(4)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let result: anyhow::Result<Ping> = client.get_json(&url).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("HTTP 503"), "error should mention status: {err}");
    }

    // ── 4xx (non-401) is not retried ─────────────────────────────────────

    #[tokio::test]
    async fn does_not_retry_on_403() {
        let server = MockServer::start().await;
        let url = format!("{}/forbidden", server.uri());

        Mock::given(method("GET"))
            .and(path("/forbidden"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1) // exactly one attempt
            .mount(&server)
            .await;

        let client = test_client("tok");
        let result: anyhow::Result<Ping> = client.get_json(&url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("HTTP 403"));
    }

    #[tokio::test]
    async fn does_not_retry_on_404() {
        let server = MockServer::start().await;
        let url = format!("{}/not-found", server.uri());

        Mock::given(method("GET"))
            .and(path("/not-found"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let result: anyhow::Result<Ping> = client.get_json(&url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("HTTP 404"));
    }

    // ── Token accessor / setter ──────────────────────────────────────────

    #[tokio::test]
    async fn set_and_get_access_token() {
        let client = DriveClient::new("initial");
        assert_eq!(client.access_token(), "initial");

        client.set_access_token("updated");
        assert_eq!(client.access_token(), "updated");
    }

    // ── No-op when no refresher attached ─────────────────────────────────

    #[tokio::test]
    async fn without_refresher_401_is_terminal() {
        let server = MockServer::start().await;
        let url = format!("{}/no-refresher", server.uri());

        Mock::given(method("GET"))
            .and(path("/no-refresher"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("expired"); // no .with_refresher()
        let result: anyhow::Result<Ping> = client.get_json(&url).await;
        assert!(result.is_err());
    }

    // ── PATCH method ──────────────────────────────────────────────────

    #[tokio::test]
    async fn patch_json_sends_and_deserialises() {
        let server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/patch-me"))
            .and(header("Authorization", "Bearer patch-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Ping {
                message: "patched".into(),
            }))
            .mount(&server)
            .await;

        let client = test_client("patch-tok");
        let body = serde_json::json!({"name": "new-name"});
        let resp: Ping = client
            .patch_json(&format!("{}/patch-me", server.uri()), &body)
            .await
            .unwrap();
        assert_eq!(resp.message, "patched");
    }

    #[tokio::test]
    async fn patch_json_retries_on_429() {
        let server = MockServer::start().await;
        let url = format!("{}/patch-429", server.uri());

        let call_count = std::sync::atomic::AtomicU32::new(0);
        Mock::given(method("PATCH"))
            .and(path("/patch-429"))
            .respond_with(move |_req: &wiremock::Request| {
                let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    ResponseTemplate::new(429)
                } else {
                    ResponseTemplate::new(200).set_body_raw(
                        r#"{"message":"ok-after-retry"}"#,
                        "application/json",
                    )
                }
            })
            .mount(&server)
            .await;

        let client = test_client("tok");
        let resp: Ping = client
            .patch_json(&url, &serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert_eq!(resp.message, "ok-after-retry");
    }

    // ── DELETE method ─────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_json_sends_token() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/delete-me"))
            .and(header("Authorization", "Bearer del-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Ping {
                message: "deleted".into(),
            }))
            .mount(&server)
            .await;

        let client = test_client("del-tok");
        let resp: Ping = client
            .delete_json(&format!("{}/delete-me", server.uri()))
            .await
            .unwrap();
        assert_eq!(resp.message, "deleted");
    }

    #[tokio::test]
    async fn delete_empty_accepts_204() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/delete-204"))
            .and(header("Authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = test_client("tok");
        client
            .delete_empty(&format!("{}/delete-204", server.uri()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_empty_fails_on_404() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/delete-404"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1) // 404 is not retried
            .mount(&server)
            .await;

        let client = test_client("tok");
        let result = client
            .delete_empty(&format!("{}/delete-404", server.uri()))
            .await;
        assert!(result.is_err());
    }

    // ── get_raw ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_raw_returns_response() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/raw"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                b"binary-content".to_vec(),
                "application/octet-stream",
            ))
            .mount(&server)
            .await;

        let client = test_client("tok");
        let resp = client
            .get_raw(&format!("{}/raw", server.uri()))
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.bytes().await.unwrap();
        assert_eq!(&body[..], b"binary-content");
    }

    // ── Token refresher with no-arg constructor ──────────────────────

    #[tokio::test]
    async fn with_refresher_attaches_correctly() {
        let refresher = Arc::new(CountingRefresher::new("refreshed"));
        let client = test_client("old").with_refresher(refresher);
        // The refresher should be attached; verify via access_token getter.
        assert_eq!(client.access_token(), "old");
    }
}
