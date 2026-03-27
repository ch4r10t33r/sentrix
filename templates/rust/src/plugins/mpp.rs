//! Machine Payments Protocol (MPP) → Borgkit Plugin (Rust)
//!
//! Adds MPP HTTP 402 payment gating to any Borgkit agent.  The plugin
//! intercepts incoming `/invoke` requests, issues challenges when no valid
//! credential is present, and attaches a `Payment-Receipt` to successful
//! responses.
//!
//! MPP flow (https://mpp.dev)
//! ──────────────────────────
//!   1. Client → POST /invoke  (no credential)
//!   2. Agent  ← 402 + `WWW-Authenticate: Payment <challenge>`
//!   3. Client pays via Tempo / Stripe / Lightning off-band
//!   4. Client → POST /invoke with `Authorization: Payment <credential>`
//!   5. Agent verifies → 200 + `Payment-Receipt: <receipt>`
//!
//! Payment methods
//! ───────────────
//!   • Tempo  — TIP-20 stablecoin on the Tempo EVM chain
//!   • Stripe — Shared Payment Tokens (SPT) via Stripe
//!
//! Dependencies (add to Cargo.toml)
//! ─────────────────────────────────
//!   # Core dependencies (already in most Borgkit Rust agents)
//!   reqwest    = { version = "0.12", default-features = false, features = ["blocking","json","rustls-tls"] }
//!   serde      = { version = "1",    features = ["derive"] }
//!   serde_json = "1"
//!   tokio      = { version = "1",    features = ["full"] }
//!   base64     = { version = "0.22", features = ["alloc"] }
//!   rand       = "0.8"
//!   thiserror  = "1"
//!   axum       = "0.7"              # if using axum integration
//!   tower      = "0.4"              # if using tower middleware
//!
//!   # Official MPP Rust SDK (optional — enhanced verification)
//!   # mpp = { version = "*", features = ["server","client","tempo","stripe"] }
//!
//! Usage — server middleware (axum)
//! ────────────────────────────────
//!   use borgkit::plugins::mpp::{MppPlugin, MppConfig, TempoConfig, MppPricing};
//!
//!   let plugin = MppPlugin::new(MppConfig {
//!       method: MppMethod::Tempo,
//!       tempo:  Some(TempoConfig {
//!           recipient: "0x742d35Cc6634c0532925a3b844Bc9e7595f1B0F2".into(),
//!           currency:  None,
//!           rpc:       None,
//!       }),
//!       pricing: MppPricing {
//!           default: "0.01".into(),
//!           per_capability: indexmap! {
//!               "summarise".into() => "0.05".into(),
//!           },
//!       },
//!       ..Default::default()
//!   })?;
//!
//!   // Axum layer
//!   let app = Router::new()
//!       .route("/invoke", post(invoke_handler))
//!       .layer(plugin.axum_layer());
//!
//! Usage — manual middleware
//! ─────────────────────────
//!   let challenge = plugin.challenge_for("summarise");  // → header value string
//!   plugin.verify_credential(&credential).await?;       // → Ok(()) or Err
//!   let receipt   = plugin.receipt_for("summarise", &nonce);
//!
//! Usage — MPP client (paying agent)
//! ─────────────────────────────────
//!   use borgkit::plugins::mpp::{MppClient, MppClientConfig, TempoClientConfig};
//!
//!   let client = MppClient::new(MppClientConfig {
//!       tempo: Some(TempoClientConfig {
//!           private_key: "0xabc...".into(),
//!           rpc:         "https://rpc.moderato.tempo.xyz".into(),
//!       }),
//!       ..Default::default()
//!   });
//!
//!   // Automatically retries with payment on 402
//!   let resp = client.post("https://agent.example.com/invoke", &payload).await?;
//!   let receipt = resp.headers().get("Payment-Receipt");

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Header constants ─────────────────────────────────────────────────────────

pub const WWW_AUTH_HEADER: &str    = "WWW-Authenticate";
pub const AUTH_HEADER: &str        = "Authorization";
pub const RECEIPT_HEADER: &str     = "Payment-Receipt";
pub const PAYMENT_SCHEME: &str     = "Payment";
pub const DEFAULT_TEMPO_CURRENCY: &str = "0x20c0000000000000000000000000000000000000";
pub const DEFAULT_TEMPO_RPC: &str      = "https://rpc.moderato.tempo.xyz";

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum MppError {
    MissingConfig(String),
    InvalidCredential(String),
    PaymentFailed(String),
    HttpError(String),
}

impl std::fmt::Display for MppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MppError::MissingConfig(s)     => write!(f, "MPP config error: {}", s),
            MppError::InvalidCredential(s) => write!(f, "MPP invalid credential: {}", s),
            MppError::PaymentFailed(s)     => write!(f, "MPP payment failed: {}", s),
            MppError::HttpError(s)         => write!(f, "MPP HTTP error: {}", s),
        }
    }
}

impl std::error::Error for MppError {}

// ── Config types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MppMethod {
    Tempo,
    Stripe,
    Lightning,
}

/// Tempo stablecoin payment configuration.
#[derive(Debug, Clone)]
pub struct TempoConfig {
    /// EVM address of the payment recipient.
    pub recipient: String,
    /// TIP-20 token contract address (default: USDC on Moderato testnet).
    pub currency: Option<String>,
    /// Tempo RPC endpoint (default: Moderato testnet).
    pub rpc: Option<String>,
}

/// Stripe Shared Payment Token configuration.
#[derive(Debug, Clone)]
pub struct StripeConfig {
    /// Stripe secret key (server-side only).
    pub secret_key: String,
    /// Network ID (default: `"internal"` for test mode).
    pub network_id: Option<String>,
    /// ISO currency code (default: `"usd"`).
    pub currency: Option<String>,
    /// Decimal places (default: `2`).
    pub decimals: Option<u8>,
}

/// Pricing schedule for the MPP server.
#[derive(Debug, Clone)]
pub struct MppPricing {
    /// Default charge for any `/invoke` call (token units as string).
    pub default: String,
    /// Per-capability overrides — capability name → amount.
    pub per_capability: HashMap<String, String>,
}

impl Default for MppPricing {
    fn default() -> Self {
        Self { default: "0.01".into(), per_capability: HashMap::new() }
    }
}

/// Full MPP plugin configuration.
#[derive(Debug, Clone)]
pub struct MppConfig {
    /// Payment method to advertise.
    pub method: MppMethod,
    /// Tempo config (required for `Tempo` method).
    pub tempo: Option<TempoConfig>,
    /// Stripe config (required for `Stripe` method).
    pub stripe: Option<StripeConfig>,
    /// Pricing schedule.
    pub pricing: MppPricing,
    /// Skip payment verification on `127.0.0.1` / `::1` (default: false).
    pub skip_on_localhost: bool,
}

impl Default for MppConfig {
    fn default() -> Self {
        Self {
            method:            MppMethod::Tempo,
            tempo:             None,
            stripe:            None,
            pricing:           MppPricing::default(),
            skip_on_localhost: false,
        }
    }
}

// ── Internal wire types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct ReceiptPayload {
    method: String,
    amount: String,
    nonce:  String,
    ts:     u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CredentialPayload {
    method: String,
    nonce:  String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spt:    Option<String>,
}

// ── Helper functions ─────────────────────────────────────────────────────────

/// Generate a random 16-byte nonce as a lowercase hex string.
pub fn generate_nonce() -> String {
    let bytes: [u8; 16] = rand::thread_rng().gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Current UNIX timestamp in milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Challenge builders ────────────────────────────────────────────────────────

fn build_tempo_challenge(cfg: &TempoConfig, amount: &str, nonce: &str) -> String {
    let currency = cfg.currency.as_deref().unwrap_or(DEFAULT_TEMPO_CURRENCY);
    let rpc      = cfg.rpc.as_deref().unwrap_or(DEFAULT_TEMPO_RPC);
    format!(
        r#"{PAYMENT_SCHEME} method="tempo", recipient="{}", currency="{}", rpc="{}", amount="{}", nonce="{}""#,
        cfg.recipient, currency, rpc, amount, nonce,
    )
}

fn build_stripe_challenge(cfg: &StripeConfig, amount: &str, nonce: &str) -> String {
    let network_id = cfg.network_id.as_deref().unwrap_or("internal");
    let currency   = cfg.currency.as_deref().unwrap_or("usd");
    let decimals   = cfg.decimals.unwrap_or(2);
    format!(
        r#"{PAYMENT_SCHEME} method="stripe", network_id="{}", currency="{}", decimals="{}", amount="{}", nonce="{}""#,
        network_id, currency, decimals, amount, nonce,
    )
}

/// Parse a `WWW-Authenticate: Payment ...` header into key-value pairs.
pub fn parse_challenge(header: &str) -> HashMap<String, String> {
    let body = header
        .strip_prefix(PAYMENT_SCHEME)
        .unwrap_or(header)
        .trim();
    let mut map = HashMap::new();
    for part in body.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches('"').to_string();
            map.insert(key, val);
        }
    }
    map
}

/// Extract the raw token from `Authorization: Payment <token>`.
pub fn extract_credential(header: &str) -> Option<&str> {
    let prefix = PAYMENT_SCHEME.to_string() + " ";
    header.strip_prefix(prefix.as_str()).map(str::trim)
}

// ── MppPlugin — server side ───────────────────────────────────────────────────

/// MPP payment middleware for Borgkit agents.
///
/// See module docs for usage examples.
pub struct MppPlugin {
    cfg: MppConfig,
}

impl MppPlugin {
    pub fn new(cfg: MppConfig) -> Result<Self, MppError> {
        if cfg.method == MppMethod::Tempo && cfg.tempo.is_none() {
            return Err(MppError::MissingConfig("tempo config required".into()));
        }
        if cfg.method == MppMethod::Stripe && cfg.stripe.is_none() {
            return Err(MppError::MissingConfig("stripe config required".into()));
        }
        Ok(Self { cfg })
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Build the `WWW-Authenticate` header value for a given capability.
    pub fn challenge_for(&self, capability: &str) -> String {
        let amount = self.price_for(capability);
        let nonce  = generate_nonce();
        self.build_challenge(&amount, &nonce)
    }

    /// Build the `WWW-Authenticate` header value with an explicit amount.
    pub fn challenge_with_amount(&self, amount: &str) -> String {
        let nonce = generate_nonce();
        self.build_challenge(amount, &nonce)
    }

    /// Verify a payment credential from the `Authorization: Payment <token>` header.
    ///
    /// In production: delegate to the mpp crate's `verify_credential`.
    /// Here we perform structural validation and return `Ok(())` for non-empty tokens
    /// (replace with on-chain/Stripe verification in production).
    pub async fn verify_credential(&self, credential: &str) -> Result<(), MppError> {
        if credential.trim().is_empty() {
            return Err(MppError::InvalidCredential("empty credential".into()));
        }
        // Attempt to decode and parse the credential JSON
        let bytes = B64.decode(credential)
            .map_err(|_| MppError::InvalidCredential("base64 decode failed".into()))?;
        let _payload: CredentialPayload = serde_json::from_slice(&bytes)
            .map_err(|_| MppError::InvalidCredential("invalid credential JSON".into()))?;
        // Production: call mpp crate or on-chain RPC here.
        Ok(())
    }

    /// Build a `Payment-Receipt` header value.
    pub fn receipt_for(&self, capability: &str, nonce: &str) -> String {
        let amount  = self.price_for(capability);
        let method  = match self.cfg.method {
            MppMethod::Tempo     => "tempo",
            MppMethod::Stripe    => "stripe",
            MppMethod::Lightning => "lightning",
        };
        let payload = ReceiptPayload {
            method: method.into(),
            amount,
            nonce:  nonce.into(),
            ts:     now_ms(),
        };
        serde_json::to_string(&payload).unwrap_or_default()
    }

    /// Price for the given capability (falls back to default).
    pub fn price_for(&self, capability: &str) -> String {
        self.cfg.pricing
            .per_capability
            .get(capability)
            .cloned()
            .unwrap_or_else(|| self.cfg.pricing.default.clone())
    }

    // ── Axum integration ─────────────────────────────────────────────────────

    /// Build an `axum::middleware::from_fn_with_state` compatible middleware function.
    ///
    /// ```rust
    /// use axum::{Router, routing::post};
    ///
    /// let app = Router::new()
    ///     .route("/invoke", post(invoke_handler))
    ///     .layer(plugin.axum_layer());
    /// ```
    #[cfg(feature = "axum")]
    pub fn axum_layer(
        self,
    ) -> axum::middleware::from_fn_with_state::FromFnLayer<
        impl Fn(
            axum::extract::State<std::sync::Arc<MppPlugin>>,
            axum::http::Request<axum::body::Body>,
            axum::middleware::Next,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response> + Send>>,
        std::sync::Arc<MppPlugin>,
        axum::body::Body,
    > {
        use axum::{
            extract::State,
            http::{Request, StatusCode},
            middleware::Next,
            response::Response,
        };
        use std::sync::Arc;

        let state = Arc::new(self);
        axum::middleware::from_fn_with_state(state, mpp_axum_middleware)
    }

    // ── Private ───────────────────────────────────────────────────────────────

    fn build_challenge(&self, amount: &str, nonce: &str) -> String {
        match self.cfg.method {
            MppMethod::Tempo     => build_tempo_challenge(self.cfg.tempo.as_ref().unwrap(), amount, nonce),
            MppMethod::Stripe    => build_stripe_challenge(self.cfg.stripe.as_ref().unwrap(), amount, nonce),
            MppMethod::Lightning => format!(
                r#"{PAYMENT_SCHEME} method="lightning", amount="{}", nonce="{}""#,
                amount, nonce,
            ),
        }
    }
}

// ── Axum middleware fn ────────────────────────────────────────────────────────

#[cfg(feature = "axum")]
async fn mpp_axum_middleware(
    State(plugin): axum::extract::State<std::sync::Arc<MppPlugin>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    // Extract Authorization header
    let auth = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| extract_credential(v).map(str::to_string));

    if let Some(credential) = auth {
        if plugin.verify_credential(&credential).await.is_ok() {
            let mut response = next.run(request).await;
            let nonce   = generate_nonce();
            let receipt = plugin.receipt_for("invoke", &nonce);
            response.headers_mut().insert(
                RECEIPT_HEADER,
                receipt.parse().unwrap_or_else(|_| "ok".parse().unwrap()),
            );
            return response;
        }
    }

    // Issue 402 challenge
    let challenge = plugin.challenge_with_amount(&plugin.cfg.pricing.default);
    (
        StatusCode::PAYMENT_REQUIRED,
        [
            (WWW_AUTH_HEADER, challenge.as_str()),
            ("Content-Type",  "application/json"),
        ],
        r#"{"error":"Payment required"}"#,
    )
        .into_response()
}

// ── Manual integration helpers ───────────────────────────────────────────────

/// Gate a standard HTTP handler: check for a valid `Authorization: Payment` header
/// and either return a 402 challenge or call the handler.
///
/// Returns `None` when the credential is valid (proceed normally).
/// Returns `Some((status, headers, body))` when a 402 should be sent.
pub async fn gate_request(
    plugin: &MppPlugin,
    auth_header: Option<&str>,
    capability: &str,
) -> Option<(u16, Vec<(String, String)>, String)> {
    if let Some(auth) = auth_header {
        if let Some(cred) = extract_credential(auth) {
            if plugin.verify_credential(cred).await.is_ok() {
                return None;   // all good
            }
        }
    }
    let challenge = plugin.challenge_for(capability);
    Some((
        402,
        vec![
            (WWW_AUTH_HEADER.into(), challenge),
            ("Content-Type".into(), "application/json".into()),
        ],
        r#"{"error":"Payment required"}"#.into(),
    ))
}

// ── MppClient — paying agent side ────────────────────────────────────────────

/// Configuration for the MPP payment client.
#[derive(Debug, Clone, Default)]
pub struct MppClientConfig {
    /// Tempo payment provider config.
    pub tempo: Option<TempoClientConfig>,
    /// Stripe payment provider config.
    pub stripe: Option<StripeClientConfig>,
    /// Maximum number of payment retries per request (default: 1).
    pub max_retries: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct TempoClientConfig {
    /// Hex-encoded private key of the paying wallet (`0x...`).
    pub private_key: String,
    /// Tempo chain RPC (default: Moderato testnet).
    pub rpc: String,
}

#[derive(Debug, Clone)]
pub struct StripeClientConfig {
    /// Backend URL that creates Stripe Shared Payment Tokens.
    pub spt_endpoint: String,
}

/// HTTP client that automatically pays MPP 402 challenges.
///
/// Uses `reqwest` under the hood.  For production use, prefer the official
/// `mpp` crate's `PaymentMiddleware` which handles on-chain signing natively.
pub struct MppClient {
    cfg:    MppClientConfig,
    http:   reqwest::Client,
    retries: u8,
}

impl MppClient {
    pub fn new(cfg: MppClientConfig) -> Self {
        let retries = cfg.max_retries.unwrap_or(1);
        Self {
            cfg,
            http: reqwest::Client::new(),
            retries,
        }
    }

    /// POST JSON to a URL, automatically handling 402 MPP challenges.
    pub async fn post<T: Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response, MppError> {
        let body_bytes = serde_json::to_vec(body)
            .map_err(|e| MppError::PaymentFailed(e.to_string()))?;
        self.post_bytes(url, body_bytes, None, 0).await
    }

    async fn post_bytes(
        &self,
        url: &str,
        body: Vec<u8>,
        credential: Option<String>,
        attempt: u8,
    ) -> Result<reqwest::Response, MppError> {
        let mut req = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.clone());

        if let Some(cred) = &credential {
            req = req.header(AUTH_HEADER, format!("{PAYMENT_SCHEME} {cred}"));
        }

        let resp = req.send().await.map_err(|e| MppError::HttpError(e.to_string()))?;

        if resp.status().as_u16() != 402 || attempt >= self.retries {
            return Ok(resp);
        }

        let challenge_header = resp
            .headers()
            .get(WWW_AUTH_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let cred = self.pay_challenge(&challenge_header).await?;
        self.post_bytes(url, body, Some(cred), attempt + 1).await
    }

    async fn pay_challenge(&self, challenge: &str) -> Result<String, MppError> {
        let params = parse_challenge(challenge);
        let method = params.get("method").map(String::as_str).unwrap_or("");

        match method {
            "tempo" => self.pay_tempo(&params).await,
            "stripe" => self.pay_stripe(&params).await,
            other => Err(MppError::PaymentFailed(format!("unsupported method: {other}"))),
        }
    }

    async fn pay_tempo(&self, params: &HashMap<String, String>) -> Result<String, MppError> {
        let cfg = self.cfg.tempo.as_ref()
            .ok_or_else(|| MppError::MissingConfig("tempo client config required".into()))?;

        // Production: sign and broadcast a TIP-20 transfer using the private key.
        // For development: generate a stub credential.
        let nonce   = params.get("nonce").cloned().unwrap_or_else(generate_nonce);
        let payload = CredentialPayload {
            method:  "tempo".into(),
            nonce:   nonce.clone(),
            tx_hash: Some(format!("0x{}", generate_nonce())),
            spt:     None,
        };
        let json  = serde_json::to_vec(&payload).unwrap_or_default();
        let token = B64.encode(json);
        let _ = cfg;  // suppress unused warning — used in production signing
        Ok(token)
    }

    async fn pay_stripe(&self, params: &HashMap<String, String>) -> Result<String, MppError> {
        let cfg = self.cfg.stripe.as_ref()
            .ok_or_else(|| MppError::MissingConfig("stripe client config required".into()))?;

        // Call the SPT endpoint to get a Shared Payment Token
        let resp = self.http
            .post(&cfg.spt_endpoint)
            .json(params)
            .send()
            .await
            .map_err(|e| MppError::HttpError(e.to_string()))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| MppError::PaymentFailed(e.to_string()))?;

        let spt = body["spt"].as_str()
            .ok_or_else(|| MppError::PaymentFailed("no spt in response".into()))?
            .to_string();

        let nonce   = params.get("nonce").cloned().unwrap_or_else(generate_nonce);
        let payload = CredentialPayload { method: "stripe".into(), nonce, tx_hash: None, spt: Some(spt) };
        let json    = serde_json::to_vec(&payload).unwrap_or_default();
        Ok(B64.encode(json))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tempo_plugin() -> MppPlugin {
        MppPlugin::new(MppConfig {
            method: MppMethod::Tempo,
            tempo: Some(TempoConfig {
                recipient: "0x742d35Cc6634c0532925a3b844Bc9e7595f1B0F2".into(),
                currency:  None,
                rpc:       None,
            }),
            pricing: MppPricing {
                default: "0.01".into(),
                per_capability: HashMap::from([("summarise".into(), "0.05".into())]),
            },
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn challenge_contains_required_fields() {
        let plugin    = tempo_plugin();
        let challenge = plugin.challenge_for("chat");
        assert!(challenge.starts_with("Payment"), "must start with Payment scheme");
        assert!(challenge.contains("method=\"tempo\""));
        assert!(challenge.contains("amount=\"0.01\""));
        assert!(challenge.contains("recipient="));
        assert!(challenge.contains("nonce="));
    }

    #[test]
    fn per_capability_pricing() {
        let plugin = tempo_plugin();
        assert_eq!(plugin.price_for("summarise"), "0.05");
        assert_eq!(plugin.price_for("other"),     "0.01");
    }

    #[tokio::test]
    async fn valid_credential_accepted() {
        let plugin = tempo_plugin();
        let payload = CredentialPayload {
            method:  "tempo".into(),
            nonce:   "abc123".into(),
            tx_hash: Some("0xdeadbeef".into()),
            spt:     None,
        };
        let token = B64.encode(serde_json::to_vec(&payload).unwrap());
        assert!(plugin.verify_credential(&token).await.is_ok());
    }

    #[tokio::test]
    async fn empty_credential_rejected() {
        let plugin = tempo_plugin();
        assert!(plugin.verify_credential("").await.is_err());
    }

    #[test]
    fn parse_challenge_roundtrip() {
        let plugin    = tempo_plugin();
        let challenge = plugin.challenge_for("chat");
        let params    = parse_challenge(&challenge);
        assert_eq!(params.get("method").unwrap(), "tempo");
        assert!(params.contains_key("nonce"));
        assert!(params.contains_key("amount"));
    }

    #[test]
    fn extract_credential_from_header() {
        let header = "Payment abc123xyz";
        assert_eq!(extract_credential(header), Some("abc123xyz"));
        assert_eq!(extract_credential("Bearer token"), None);
    }

    #[test]
    fn receipt_is_valid_json() {
        let plugin  = tempo_plugin();
        let receipt = plugin.receipt_for("summarise", "testnonce");
        let parsed: serde_json::Value = serde_json::from_str(&receipt).unwrap();
        assert_eq!(parsed["method"].as_str().unwrap(), "tempo");
        assert_eq!(parsed["amount"].as_str().unwrap(), "0.05");
        assert_eq!(parsed["nonce"].as_str().unwrap(), "testnonce");
    }
}
