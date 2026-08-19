use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use reqwest::cookie::CookieStore;
use reqwest::Url;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::warn;

/// Shared User-Agent header used for all NSE API/WSS requests. NSE's edge
/// (both REST and the WSS handshake) rejects requests from clients that
/// don't look like a browser, so this must match across HTTP and WS calls.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// The plain HTTPS origin NSE expects to see cookies/Origin/Referer for. Used
/// both to warm the cookie jar and to stamp the WSS upgrade request.
pub const NSE_ORIGIN: &str = "https://www.nseindia.com";

/// App-lifetime, shared cookie jar backing `HTTP_CLIENT`. Kept separately
/// (rather than only inside the client) so streamers can read the current
/// cookie values to attach to the WSS handshake, which `reqwest` itself
/// never performs.
static COOKIE_JAR: OnceLock<Arc<reqwest::cookie::Jar>> = OnceLock::new();

/// App-lifetime, shared `reqwest::Client`. Cheap to clone (Arc-backed), reused
/// across concurrent tasks so TCP/TLS connections to www.nseindia.com get
/// pooled. Cookie storage is enabled so the session cookies NSE sets on the
/// first hit to the site are retained and replayed on subsequent requests.
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn jar() -> Arc<reqwest::cookie::Jar> {
    COOKIE_JAR
        .get_or_init(|| Arc::new(reqwest::cookie::Jar::default()))
        .clone()
}

pub fn get_client() -> reqwest::Client {
    HTTP_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .cookie_provider(jar())
                .build()
                .unwrap_or_default()
        })
        .clone()
}

pub fn get(url: &str) -> reqwest::RequestBuilder {
    get_client().get(url).header("User-Agent", USER_AGENT)
}

/// NSE (both the REST API and the streamer WSS endpoints) rejects requests
/// that don't carry a valid session cookie obtained by first visiting the
/// plain website. Hit the homepage so the jar picks up the `nsit`/`nseappid`
/// (and similar) cookies NSE issues, then reuse them for later WSS
/// handshakes via [`cookie_header_for`].
///
/// Safe to call repeatedly; NSE's cookies are short-lived, so callers should
/// re-warm before each reconnect attempt rather than caching this at
/// startup.
pub async fn warm_nse_session() -> Result<()> {
    let resp = get_client()
        .get(NSE_ORIGIN)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| anyhow!("Failed to warm NSE session cookies: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "NSE session warmup returned unexpected status: {}",
            resp.status()
        ));
    }

    Ok(())
}

/// Read back the current `Cookie:` header value (semicolon-joined
/// `name=value` pairs) that would be sent to `url`, as populated by
/// [`warm_nse_session`]. Returns an error if no cookies are held yet (i.e.
/// warmup hasn't happened or NSE didn't set any).
pub fn cookie_header_for(url: &str) -> Result<String> {
    let parsed = Url::parse(url).map_err(|e| anyhow!("Invalid URL for cookie lookup: {}", e))?;
    let header = jar()
        .cookies(&parsed)
        .ok_or_else(|| anyhow!("No NSE session cookies available; call warm_nse_session() first"))?;

    header
        .to_str()
        .map(|s| s.to_string())
        .map_err(|e| anyhow!("NSE cookie header is not valid UTF-8/ASCII: {}", e))
}

/// Connect to an NSE streamer WSS endpoint.
///
/// `with_browser_headers` controls whether `Origin`/`Referer`/`User-Agent`
/// (and, if available, a `Cookie` from a best-effort session warmup against
/// `https://www.nseindia.com`) are attached to the WS upgrade. Pass `true`
/// only for endpoints confirmed to need it (currently: indices) — the
/// options endpoint was working with a bare connect before, so it keeps
/// that exact behavior (`false`) to avoid regressing it.
///
/// Cookie warmup/lookup is always best-effort: failures (network error, no
/// cookies set, host-scoped cookie mismatch between `www.nseindia.com` and
/// `streamer.nseindia.com`, etc.) just mean the `Cookie` header is omitted,
/// never a hard failure of the connection attempt.
pub async fn connect_nse_ws(
    url: &str,
    with_browser_headers: bool,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let mut request = url
        .into_client_request()
        .map_err(|e| anyhow!("Invalid WSS URL {}: {}", url, e))?;

    if with_browser_headers {
        if let Err(e) = warm_nse_session().await {
            warn!("NSE session warmup failed, connecting without cookies: {}", e);
        }
        let cookie = cookie_header_for(url).ok();

        let headers = request.headers_mut();
        headers.insert(
            "User-Agent",
            HeaderValue::from_str(USER_AGENT)
                .map_err(|e| anyhow!("Bad User-Agent header: {}", e))?,
        );
        headers.insert(
            "Origin",
            HeaderValue::from_str(NSE_ORIGIN).map_err(|e| anyhow!("Bad Origin header: {}", e))?,
        );
        headers.insert(
            "Referer",
            HeaderValue::from_str(&format!("{}/", NSE_ORIGIN))
                .map_err(|e| anyhow!("Bad Referer header: {}", e))?,
        );
        if let Some(cookie) = cookie {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                headers.insert("Cookie", value);
            }
        }
    }

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| anyhow!("WebSocket handshake failed: {}", e))?;

    Ok(ws_stream)
}