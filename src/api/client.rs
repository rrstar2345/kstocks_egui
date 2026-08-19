//! Thin HTTP client for the kstocks-server read-only API. Mirrors the
//! request/response shapes documented in the server's README so this stays
//! a straightforward reflection of the server contract, not a reinterpretation
//! of it.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

// ============================================================================
// RESPONSE / REQUEST TYPES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub status: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResponse {
    pub approved: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub db_connected: bool,
    pub last_index_tick_at: Option<String>,
    pub last_option_tick_at: Option<String>,
    pub aggregation_watermarks: serde_json::Value,
    pub session_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBar {
    pub bucket_start: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionBar {
    pub bucket_start: String,
    pub ce_open: Option<f64>,
    pub ce_high: Option<f64>,
    pub ce_low: Option<f64>,
    pub ce_close: Option<f64>,
    pub ce_volume: Option<i64>,
    pub ce_oi_close: Option<f64>,
    pub pe_open: Option<f64>,
    pub pe_high: Option<f64>,
    pub pe_low: Option<f64>,
    pub pe_close: Option<f64>,
    pub pe_volume: Option<i64>,
    pub pe_oi_close: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("request failed: {0}")]
    Request(String),
    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },
    #[error("no API key configured; register first")]
    MissingApiKey,
}

impl ApiClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build reqwest client");
        Self { base_url, api_key, http }
    }

    pub fn set_api_key(&mut self, api_key: Option<String>) {
        self.api_key = api_key;
    }

    pub fn set_base_url(&mut self, base_url: String) {
        self.base_url = base_url;
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth_header(&self) -> Result<String, ApiError> {
        self.api_key
            .clone()
            .map(|k| format!("Bearer {}", k))
            .ok_or(ApiError::MissingApiKey)
    }

    async fn handle_response<T: for<'de> Deserialize<'de>>(
        resp: reqwest::Response,
    ) -> Result<T, ApiError> {
        let status = resp.status();
        let text = resp.text().await.map_err(|e| ApiError::Request(e.to_string()))?;

        if status.is_success() {
            serde_json::from_str::<T>(&text).map_err(|e| ApiError::Request(e.to_string()))
        } else {
            let message = serde_json::from_str::<ApiErrorBody>(&text)
                .map(|b| b.error)
                .unwrap_or(text);
            Err(ApiError::Server { status: status.as_u16(), message })
        }
    }

    /// `POST /register` — no auth required. Called once by a new client
    /// install; the returned `api_key` should be persisted immediately.
    pub async fn register(&self, username: &str) -> Result<RegisterResponse, ApiError> {
        let resp = self
            .http
            .post(self.url("/register"))
            .json(&RegisterRequest { username: username.to_string() })
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;
        Self::handle_response(resp).await
    }

    /// `GET /validate` — checks the stored API key's current approval
    /// status. Call on every app launch.
    pub async fn validate(&self) -> Result<ValidateResponse, ApiError> {
        let resp = self
            .http
            .get(self.url("/validate"))
            .header("Authorization", self.auth_header()?)
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;
        Self::handle_response(resp).await
    }

    /// `GET /health` — no auth required. Ingest/aggregation status.
    pub async fn health(&self) -> Result<HealthResponse, ApiError> {
        let resp = self
            .http
            .get(self.url("/health"))
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;
        Self::handle_response(resp).await
    }

    /// `GET /ohlc/index?symbol=...&range=...&interval=...`
    pub async fn ohlc_index(
        &self,
        symbol: &str,
        range: &str,
        interval: &str,
    ) -> Result<Vec<IndexBar>, ApiError> {
        let resp = self
            .http
            .get(self.url("/ohlc/index"))
            .header("Authorization", self.auth_header()?)
            .query(&[("symbol", symbol), ("range", range), ("interval", interval)])
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;
        Self::handle_response(resp).await
    }

    /// `GET /ohlc/option?symbol=...&expiry=...&strike=...&range=...&interval=...&leg=...`
    #[allow(clippy::too_many_arguments)]
    pub async fn ohlc_option(
        &self,
        symbol: &str,
        expiry: &str,
        strike: f64,
        range: &str,
        interval: &str,
        leg: &str,
    ) -> Result<Vec<OptionBar>, ApiError> {
        let resp = self
            .http
            .get(self.url("/ohlc/option"))
            .header("Authorization", self.auth_header()?)
            .query(&[
                ("symbol", symbol),
                ("expiry", expiry),
                ("strike", &strike.to_string()),
                ("range", range),
                ("interval", interval),
                ("leg", leg),
            ])
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;
        Self::handle_response(resp).await
    }
}
