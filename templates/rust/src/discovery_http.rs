/*!
HttpDiscovery — centralised discovery adapter (optional extension).

Connects to any REST-based agent registry that implements the Borgkit
centralised discovery API. This is NOT the default; LocalDiscovery and
GossipDiscovery are preferred.

The server must expose:
  POST   /agents           → register
  DELETE /agents/{id}      → unregister
  GET    /agents?cap=X     → query by capability
  GET    /agents           → list all
  PUT    /agents/{id}/hb   → heartbeat

Deps (Cargo.toml): reqwest, serde_json, tokio
*/

use crate::discovery::{IAgentDiscovery, DiscoveryEntry};
use crate::discovery_libp2p::{Libp2pDiscovery, Libp2pDiscoveryConfig};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct HttpDiscovery {
    base_url:     String,
    client:       Client,
    heartbeat_ms: u64,
    /// task handles keyed by agent_id (aborted on unregister)
    hb_tasks:     Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl HttpDiscovery {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_options(base_url, None, 5_000, 30_000)
    }

    pub fn with_options(
        base_url: impl Into<String>,
        api_key: Option<&str>,
        timeout_ms: u64,
        heartbeat_ms: u64,
    ) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = api_key {
            headers.insert("X-Api-Key", key.parse().unwrap());
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .expect("failed to build HTTP client");

        Self {
            base_url:     base_url.into().trim_end_matches('/').to_string(),
            client,
            heartbeat_ms,
            hb_tasks:     Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create from the BORGKIT_DISCOVERY_URL environment variable.
    pub fn from_env() -> Option<Self> {
        std::env::var("BORGKIT_DISCOVERY_URL").ok().map(|url| {
            let key = std::env::var("BORGKIT_DISCOVERY_KEY").ok();
            Self::with_options(url, key.as_deref(), 5_000, 30_000)
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait]
impl IAgentDiscovery for HttpDiscovery {
    async fn register(&self, entry: DiscoveryEntry) -> Result<(), Box<dyn std::error::Error>> {
        let body = serde_json::to_value(&entry)?;
        self.client.post(self.url("/agents"))
            .json(&body)
            .send().await?
            .error_for_status()?;

        // Start heartbeat task
        if self.heartbeat_ms > 0 {
            let client  = self.client.clone();
            let url     = self.url(&format!("/agents/{}/hb", urlencoding::encode(&entry.agent_id)));
            let ms      = self.heartbeat_ms;
            let handle  = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    if let Err(e) = client.put(&url).send().await {
                        eprintln!("[HttpDiscovery] heartbeat failed: {e}");
                    }
                }
            });
            self.hb_tasks.lock().unwrap().insert(entry.agent_id.clone(), handle);
        }

        println!("[HttpDiscovery] Registered: {} → {}", entry.agent_id, self.base_url);
        Ok(())
    }

    async fn unregister(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Abort heartbeat
        if let Some(handle) = self.hb_tasks.lock().unwrap().remove(agent_id) {
            handle.abort();
        }
        self.client
            .delete(self.url(&format!("/agents/{}", urlencoding::encode(agent_id))))
            .send().await?
            .error_for_status()?;
        Ok(())
    }

    async fn query(&self, capability: &str) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let entries: Vec<DiscoveryEntry> = self.client
            .get(self.url(&format!("/agents?cap={}", urlencoding::encode(capability))))
            .send().await?
            .error_for_status()?
            .json().await?;
        Ok(entries)
    }

    async fn list_all(&self) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let entries: Vec<DiscoveryEntry> = self.client
            .get(self.url("/agents"))
            .send().await?
            .error_for_status()?
            .json().await?;
        Ok(entries)
    }

    async fn heartbeat(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.client
            .put(self.url(&format!("/agents/{}/hb", urlencoding::encode(agent_id))))
            .send().await?
            .error_for_status()?;
        Ok(())
    }
}

// ── DiscoveryFactory ──────────────────────────────────────────────────────────

use crate::discovery::LocalDiscovery;

pub enum AnyDiscovery {
    Local(LocalDiscovery),
    Http(HttpDiscovery),
    Libp2p(Libp2pDiscovery),
}

#[async_trait]
impl IAgentDiscovery for AnyDiscovery {
    async fn register(&self, e: DiscoveryEntry) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Local(d)  => d.register(e).await,
            Self::Http(d)   => d.register(e).await,
            Self::Libp2p(d) => d.register(e).await,
        }
    }
    async fn unregister(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Local(d)  => d.unregister(id).await,
            Self::Http(d)   => d.unregister(id).await,
            Self::Libp2p(d) => d.unregister(id).await,
        }
    }
    async fn query(&self, cap: &str) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        match self {
            Self::Local(d)  => d.query(cap).await,
            Self::Http(d)   => d.query(cap).await,
            Self::Libp2p(d) => d.query(cap).await,
        }
    }
    async fn list_all(&self) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        match self {
            Self::Local(d)  => d.list_all().await,
            Self::Http(d)   => d.list_all().await,
            Self::Libp2p(d) => d.list_all().await,
        }
    }
    async fn heartbeat(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Local(d)  => d.heartbeat(id).await,
            Self::Http(d)   => d.heartbeat(id).await,
            Self::Libp2p(d) => d.heartbeat(id).await,
        }
    }
}

/// Build a discovery backend from environment or explicit config.
///
/// ```rust
/// let registry = DiscoveryFactory::from_env()
///   .unwrap_or_else(|| DiscoveryFactory::local());
/// ```
pub struct DiscoveryFactory;

impl DiscoveryFactory {
    /// Use in-memory local registry (default / dev).
    pub fn local() -> AnyDiscovery {
        AnyDiscovery::Local(LocalDiscovery::default())
    }

    /// Use centralised HTTP registry.
    pub fn http(base_url: impl Into<String>) -> AnyDiscovery {
        AnyDiscovery::Http(HttpDiscovery::new(base_url))
    }

    /// Auto-select: HTTP if BORGKIT_DISCOVERY_URL is set, else Local.
    /// Prefer `from_env_async` for new code — it defaults to libp2p.
    pub fn from_env() -> AnyDiscovery {
        HttpDiscovery::from_env()
            .map(AnyDiscovery::Http)
            .unwrap_or_else(|| AnyDiscovery::Local(LocalDiscovery::default()))
    }

    /// Use libp2p P2P discovery (QUIC + Kademlia DHT).
    pub async fn libp2p(cfg: Libp2pDiscoveryConfig) -> Result<AnyDiscovery, Box<dyn std::error::Error + Send + Sync>> {
        Ok(AnyDiscovery::Libp2p(Libp2pDiscovery::start(cfg).await?))
    }

    /// Auto-select from environment (recommended).
    ///
    /// Priority:
    ///   1. `BORGKIT_DISCOVERY_TYPE` = "local" | "http" | "libp2p"
    ///   2. `BORGKIT_DISCOVERY_URL` set → http
    ///   3. default → libp2p (falls back to local if libp2p fails to bind)
    pub async fn from_env_async() -> AnyDiscovery {
        let dtype = std::env::var("BORGKIT_DISCOVERY_TYPE").unwrap_or_default();
        match dtype.as_str() {
            "local" => return AnyDiscovery::Local(LocalDiscovery::default()),
            "http"  => return Self::from_env(),
            "libp2p" | _ if dtype.is_empty() => {
                // libp2p is the default; fall through to attempt below
            }
            _ => {
                eprintln!(
                    "[DiscoveryFactory] Unknown BORGKIT_DISCOVERY_TYPE '{}', defaulting to libp2p",
                    dtype
                );
            }
        }

        // Honour legacy BORGKIT_DISCOVERY_URL shorthand
        if dtype.is_empty() {
            if let Some(d) = HttpDiscovery::from_env() {
                return AnyDiscovery::Http(d);
            }
        }

        // Attempt libp2p; fall back to local on failure
        let cfg = Libp2pDiscoveryConfig::default();
        match Libp2pDiscovery::start(cfg).await {
            Ok(d) => AnyDiscovery::Libp2p(d),
            Err(e) => {
                eprintln!(
                    "[DiscoveryFactory] libp2p failed to start ({e}); \
                     falling back to LocalDiscovery. \
                     Set BORGKIT_DISCOVERY_TYPE=local to suppress this warning."
                );
                AnyDiscovery::Local(LocalDiscovery::default())
            }
        }
    }
}
