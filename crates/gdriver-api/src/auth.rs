//! Google OAuth 2.0 + PKCE implementation for gDriver.
//!
//! ## Typical flow
//!
//! 1. [`generate_pkce`] — create a verifier/challenge pair.
//! 2. [`start_callback_server`] — bind a local HTTP listener; get the port.
//! 3. [`build_auth_url`] — build the Google authorization URL using that port.
//! 4. Open `auth_url` in the system browser (the daemon does not do this itself).
//! 5. Await the `oneshot::Receiver` from step 2 to get the [`AuthCallback`].
//! 6. [`exchange_code`] — POST the code + verifier to the token endpoint.
//! 7. Persist the resulting [`TokenSet`] via the keyring (M2.2).
//! 8. Call [`refresh_access_token`] whenever [`TokenSet::should_refresh`] is `true`.
//!
//! For convenience, [`begin_auth_flow`] orchestrates steps 1–6 in one call.

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use tracing::{debug, info};

// ─── PKCE ─────────────────────────────────────────────────────────────────────

/// PKCE code-verifier and derived code-challenge (S256 method).
///
/// Per RFC 7636:
///   `code_verifier`  = 64 random bytes, base64url-encoded (no padding) → 86 chars
///   `code_challenge` = BASE64URL(SHA-256(ASCII(code_verifier)))        → 43 chars
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
}

/// Generate a fresh PKCE verifier / challenge pair.
pub fn generate_pkce() -> PkceChallenge {
    let mut raw = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut raw);
    let code_verifier = URL_SAFE_NO_PAD.encode(raw);

    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest.as_slice());

    PkceChallenge {
        code_verifier,
        code_challenge,
    }
}

// ─── OAuth configuration ──────────────────────────────────────────────────────

/// Google OAuth 2.0 client credentials.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl OAuthConfig {
    /// Load credentials from `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` env vars.
    pub fn from_env() -> anyhow::Result<Self> {
        let client_id = std::env::var("GOOGLE_CLIENT_ID").context("GOOGLE_CLIENT_ID not set")?;
        let client_secret =
            std::env::var("GOOGLE_CLIENT_SECRET").context("GOOGLE_CLIENT_SECRET not set")?;
        Ok(Self {
            client_id,
            client_secret,
        })
    }
}

// ─── Auth URL ─────────────────────────────────────────────────────────────────

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// OAuth scopes requested from Google.
const SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/photoslibrary",
];

/// Build the Google OAuth2 authorization URL with PKCE and offline access.
///
/// `state` is a random nonce the caller must verify in the callback to
/// prevent CSRF attacks.
pub fn build_auth_url(
    config: &OAuthConfig,
    pkce: &PkceChallenge,
    redirect_uri: &str,
    state: &str,
) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(AUTH_ENDPOINT)?;

    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("code_challenge", &pkce.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state);

    Ok(url.to_string())
}

// ─── Local callback server ────────────────────────────────────────────────────

/// Authorization code and state nonce extracted from the OAuth redirect.
#[derive(Debug, Clone)]
pub struct AuthCallback {
    pub code: String,
    pub state: String,
}

/// Bind a TCP listener on a random local port and spawn a background task that
/// accepts exactly one HTTP connection (the browser OAuth redirect), extracts
/// the authorization code, returns a success page to the browser, then exits.
///
/// Returns:
/// - `port` — use this to construct `redirect_uri = http://127.0.0.1:{port}/callback`
/// - `rx`   — resolves to `AuthCallback` (or an error) once the redirect arrives
pub async fn start_callback_server(
) -> anyhow::Result<(u16, oneshot::Receiver<anyhow::Result<AuthCallback>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind OAuth callback listener")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let result = handle_callback(listener).await;
        let _ = tx.send(result);
    });

    info!("OAuth callback server listening on 127.0.0.1:{port}");
    Ok((port, rx))
}

/// Accept one HTTP connection, parse the OAuth callback parameters, and reply.
async fn handle_callback(listener: TcpListener) -> anyhow::Result<AuthCallback> {
    let (mut stream, peer) = listener
        .accept()
        .await
        .context("OAuth callback accept failed")?;
    debug!("OAuth callback connection from {peer}");

    // Read until we see the HTTP header boundary (\r\n\r\n).
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 65_536 {
            return Err(anyhow!("OAuth callback request too large"));
        }
    }

    // Parse first line: "GET /callback?code=...&state=... HTTP/1.1"
    let request_text = std::str::from_utf8(&buf).context("callback request is not UTF-8")?;
    let request_line = request_text
        .lines()
        .next()
        .context("callback request is empty")?;

    let path = request_line
        .split_whitespace()
        .nth(1)
        .context("malformed callback request line")?;

    // Reconstruct a full URL so we can use the url crate's query-pair parser
    // (handles percent-decoding correctly, including %2B in authorization codes).
    let full_url = format!("http://localhost{path}");
    let parsed = reqwest::Url::parse(&full_url).context("failed to parse callback path")?;

    let mut code: Option<String> = None;
    let mut state = String::new();
    let mut oauth_error: Option<String> = None;

    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = value.into_owned(),
            "error" => oauth_error = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = oauth_error {
        let _ = send_html_response(&mut stream, false).await;
        return Err(anyhow!("Google OAuth denied: {err}"));
    }

    let code = code.context("no authorization code in OAuth callback")?;

    // Reply to the browser before returning so the tab shows a success page.
    let _ = send_html_response(&mut stream, true).await;

    Ok(AuthCallback { code, state })
}

async fn send_html_response(
    stream: &mut tokio::net::TcpStream,
    success: bool,
) -> std::io::Result<()> {
    let body = if success {
        "<!DOCTYPE html><html><head><meta charset=\"UTF-8\">\
         <title>gDriver — Authorized</title></head><body>\
         <h1>Authorization successful!</h1>\
         <p>You can close this window and return to gDriver.</p>\
         </body></html>"
    } else {
        "<!DOCTYPE html><html><head><meta charset=\"UTF-8\">\
         <title>gDriver — Authorization Failed</title></head><body>\
         <h1>Authorization was denied.</h1>\
         <p>You can close this window and return to gDriver.</p>\
         </body></html>"
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

// ─── Token types ──────────────────────────────────────────────────────────────

/// Raw response body from the Google token endpoint.
#[derive(Debug, Deserialize)]
struct RawTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    /// Access-token lifetime in seconds.
    expires_in: u64,
    #[allow(dead_code)]
    token_type: String,
}

/// An access/refresh token pair with a calculated expiry timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    /// `None` when a refresh response did not issue a new refresh token.
    pub refresh_token: Option<String>,
    /// Unix milliseconds when the access token expires.
    pub expires_at: i64,
}

impl TokenSet {
    /// Return `true` when the access token will expire within 5 minutes.
    ///
    /// The daemon should call [`refresh_access_token`] proactively before any
    /// API call when this returns `true`.
    pub fn should_refresh(&self) -> bool {
        let now_ms = Utc::now().timestamp_millis();
        now_ms >= self.expires_at - 5 * 60 * 1_000
    }
}

impl From<RawTokenResponse> for TokenSet {
    fn from(r: RawTokenResponse) -> Self {
        let expires_at = Utc::now().timestamp_millis() + r.expires_in as i64 * 1_000;
        Self {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            expires_at,
        }
    }
}

// ─── Token exchange ───────────────────────────────────────────────────────────

/// Exchange an authorization code for a [`TokenSet`] (PKCE flow).
pub async fn exchange_code(
    http: &Client,
    config: &OAuthConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<TokenSet> {
    debug!("exchanging authorization code for tokens");

    let resp = http
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("code", code),
            ("code_verifier", code_verifier),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .context("token exchange HTTP request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token exchange failed (HTTP {status}): {body}"));
    }

    let raw: RawTokenResponse = resp
        .json()
        .await
        .context("failed to parse token exchange response")?;
    Ok(TokenSet::from(raw))
}

// ─── Token refresh ────────────────────────────────────────────────────────────

/// Use a refresh token to obtain a new access token.
///
/// Google may not issue a new refresh token on every refresh; the original
/// `refresh_token` is preserved in the returned [`TokenSet`] when that
/// happens.  Callers must persist any new refresh token they receive.
pub async fn refresh_access_token(
    http: &Client,
    config: &OAuthConfig,
    refresh_token: &str,
) -> anyhow::Result<TokenSet> {
    debug!("refreshing access token");

    let resp = http
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("refresh_token", refresh_token),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .context("token refresh HTTP request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token refresh failed (HTTP {status}): {body}"));
    }

    let mut raw: RawTokenResponse = resp
        .json()
        .await
        .context("failed to parse token refresh response")?;

    // Preserve the existing refresh token when Google does not rotate it.
    if raw.refresh_token.is_none() {
        raw.refresh_token = Some(refresh_token.to_string());
    }

    Ok(TokenSet::from(raw))
}

// ─── High-level flow helper ───────────────────────────────────────────────────

/// Orchestrate steps 1–6 of the PKCE auth flow in a single call.
///
/// Returns immediately with `(auth_url, token_future)`.
/// The caller should open `auth_url` in the system browser, then `.await`
/// `token_future` to receive the final [`TokenSet`].
pub async fn begin_auth_flow(
    config: OAuthConfig,
    http: Client,
) -> anyhow::Result<(
    String,
    impl std::future::Future<Output = anyhow::Result<TokenSet>> + Send,
)> {
    let pkce = generate_pkce();

    let mut state_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);

    let (port, callback_rx) = start_callback_server().await?;
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let auth_url = build_auth_url(&config, &pkce, &redirect_uri, &state)?;
    let code_verifier = pkce.code_verifier.clone();

    let token_future = async move {
        let callback = callback_rx
            .await
            .context("OAuth callback server task was dropped")?
            .context("OAuth callback server error")?;

        if callback.state != state {
            return Err(anyhow!(
                "OAuth state mismatch — possible CSRF attack (expected {}, got {})",
                state,
                callback.state
            ));
        }

        exchange_code(
            &http,
            &config,
            &callback.code,
            &code_verifier,
            &redirect_uri,
        )
        .await
    };

    Ok((auth_url, token_future))
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PKCE ─────────────────────────────────────────────────────────────────

    #[test]
    fn pkce_verifier_is_86_chars() {
        // 64 random bytes → base64url (no padding) = ceil(64*4/3) = 86 chars
        let pkce = generate_pkce();
        assert_eq!(pkce.code_verifier.len(), 86, "verifier length");
    }

    #[test]
    fn pkce_challenge_is_43_chars() {
        // SHA-256 → 32 bytes → base64url (no padding) = 43 chars
        let pkce = generate_pkce();
        assert_eq!(pkce.code_challenge.len(), 43, "challenge length");
    }

    #[test]
    fn pkce_challenge_derived_correctly() {
        let pkce = generate_pkce();
        let digest = Sha256::digest(pkce.code_verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest.as_slice());
        assert_eq!(pkce.code_challenge, expected);
    }

    #[test]
    fn pkce_verifier_uses_base64url_alphabet() {
        // base64url chars: A-Z a-z 0-9 - _  (no + / or padding =)
        let pkce = generate_pkce();
        assert!(
            pkce.code_verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier contains non-base64url characters"
        );
    }

    #[test]
    fn pkce_pairs_are_unique() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_ne!(a.code_verifier, b.code_verifier);
        assert_ne!(a.code_challenge, b.code_challenge);
    }

    // ── Auth URL ─────────────────────────────────────────────────────────────

    #[test]
    fn build_auth_url_contains_required_params() {
        let config = OAuthConfig {
            client_id: "test_client_id".into(),
            client_secret: "secret".into(),
        };
        let pkce = generate_pkce();
        let url = build_auth_url(
            &config,
            &pkce,
            "http://127.0.0.1:54321/callback",
            "state_nonce_123",
        )
        .unwrap();

        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=test_client_id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=state_nonce_123"));
        assert!(url.contains(&pkce.code_challenge));
    }

    #[test]
    fn build_auth_url_includes_drive_and_photos_scopes() {
        let config = OAuthConfig {
            client_id: "cid".into(),
            client_secret: "cs".into(),
        };
        let pkce = generate_pkce();
        let url = build_auth_url(&config, &pkce, "http://127.0.0.1:0/callback", "st").unwrap();

        // Scopes are percent-encoded in the URL; verify the raw scope strings appear
        assert!(url.contains("drive"), "drive scope missing");
        assert!(url.contains("photoslibrary"), "photoslibrary scope missing");
    }

    // ── TokenSet ─────────────────────────────────────────────────────────────

    #[test]
    fn should_refresh_when_expired() {
        let t = TokenSet {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Utc::now().timestamp_millis() - 1_000, // 1 second ago
        };
        assert!(t.should_refresh());
    }

    #[test]
    fn should_not_refresh_when_fresh() {
        let t = TokenSet {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Utc::now().timestamp_millis() + 10 * 60 * 1_000, // 10 min ahead
        };
        assert!(!t.should_refresh());
    }

    #[test]
    fn should_refresh_within_5_minute_window() {
        let t = TokenSet {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Utc::now().timestamp_millis() + 3 * 60 * 1_000, // 3 min ahead
        };
        assert!(
            t.should_refresh(),
            "should proactively refresh within 5 min window"
        );
    }

    // ── Callback server ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn callback_server_extracts_code_and_state() {
        let (port, rx) = start_callback_server().await.unwrap();

        // Simulate the browser making the OAuth redirect request.
        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = "GET /callback?code=auth_code_abc&state=nonce_xyz HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       Connection: close\r\n\
                       \r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            // Read response to avoid broken-pipe on the server side.
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).await.unwrap();
        });

        let cb = rx.await.unwrap().unwrap();
        assert_eq!(cb.code, "auth_code_abc");
        assert_eq!(cb.state, "nonce_xyz");
    }

    #[tokio::test]
    async fn callback_server_returns_error_on_oauth_denied() {
        let (port, rx) = start_callback_server().await.unwrap();

        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = "GET /callback?error=access_denied&state=nonce HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       Connection: close\r\n\
                       \r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).await.unwrap();
        });

        let result = rx.await.unwrap();
        assert!(result.is_err(), "expected error on access_denied");
        assert!(result.unwrap_err().to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn callback_server_handles_percent_encoded_code() {
        let (port, rx) = start_callback_server().await.unwrap();

        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            // Authorization codes sometimes contain '/' which is percent-encoded as %2F
            let req = "GET /callback?code=4%2F0AcvDMrBa_test&state=s HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       Connection: close\r\n\
                       \r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).await.unwrap();
        });

        let cb = rx.await.unwrap().unwrap();
        // URL parser should decode %2F → /
        assert_eq!(cb.code, "4/0AcvDMrBa_test");
    }

    // ── TokenSet edge case tests ────────────────────────────────────────────

    #[test]
    fn raw_token_response_to_token_set_preserves_fields() {
        let raw = RawTokenResponse {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_in: 3600,
            token_type: "Bearer".into(),
        };
        let ts = TokenSet::from(raw);
        assert_eq!(ts.access_token, "at");
        assert_eq!(ts.refresh_token.unwrap(), "rt");
        // expires_at should be ~1 hour from now (± a few seconds tolerance).
        let now_ms = Utc::now().timestamp_millis();
        let expected = now_ms + 3600 * 1000;
        assert!(
            (ts.expires_at - expected).abs() < 5000,
            "expires_at {exp} should be close to {expected}",
            exp = ts.expires_at
        );
    }

    #[test]
    fn token_set_without_refresh_token() {
        let raw = RawTokenResponse {
            access_token: "at-only".into(),
            refresh_token: None,
            expires_in: 1800,
            token_type: "Bearer".into(),
        };
        let ts = TokenSet::from(raw);
        assert_eq!(ts.access_token, "at-only");
        assert!(ts.refresh_token.is_none());
    }

    #[test]
    fn token_set_with_zero_expiry_already_expired() {
        let raw = RawTokenResponse {
            access_token: "expired".into(),
            refresh_token: None,
            expires_in: 0,
            token_type: "Bearer".into(),
        };
        let ts = TokenSet::from(raw);
        assert!(
            ts.should_refresh(),
            "token with 0-second lifetime should need refresh"
        );
    }

    #[test]
    fn token_set_with_very_short_expiry() {
        let raw = RawTokenResponse {
            access_token: "short".into(),
            refresh_token: None,
            expires_in: 1, // 1 second
            token_type: "Bearer".into(),
        };
        let ts = TokenSet::from(raw);
        assert!(
            ts.should_refresh(),
            "token with 1s lifetime should need refresh"
        );
    }

    #[test]
    fn token_set_with_long_expiry_does_not_need_refresh() {
        let raw = RawTokenResponse {
            access_token: "long".into(),
            refresh_token: None,
            expires_in: 7200, // 2 hours
            token_type: "Bearer".into(),
        };
        let ts = TokenSet::from(raw);
        assert!(
            !ts.should_refresh(),
            "token with 2h lifetime should NOT need refresh"
        );
    }
}
