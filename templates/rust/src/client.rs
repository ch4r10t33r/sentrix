//! AgentClient — HTTP transport client for calling other Borgkit agents.
//!
//! Combines lookup (IAgentDiscovery) with invocation (HTTP POST /invoke).
//! Optional x402 payment handling built in.
//!
//! # Example
//! ```rust
//! let discovery = Arc::new(LocalDiscovery::default());
//! let client    = AgentClient::new(discovery, Default::default());
//!
//! // Discover-and-call in one step:
//! let resp = client.call_capability("weather_forecast", json!({"city": "NYC"})).await?;
//!
//! // Call a specific agent:
//! let resp = client.call("borgkit://agent/0xABC", "weather_forecast", json!({"city": "NYC"})).await?;
//! ```

use crate::discovery::{DiscoveryEntry, IAgentDiscovery};
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

// ── options ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentClientOptions {
    pub caller_id:  String,
    pub timeout_ms: u64,
}

impl Default for AgentClientOptions {
    fn default() -> Self {
        Self {
            caller_id:  "anonymous".to_string(),
            timeout_ms: 30_000,
        }
    }
}

// ── call options (per-call override) ─────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct CallOptions {
    pub caller_id:  Option<String>,
    pub timeout_ms: Option<u64>,
}

// ── trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait IAgentClient: Send + Sync {
    // ── lookup ─────────────────────────────────────────────────────────────
    async fn find(&self, capability: &str)
        -> Result<Option<DiscoveryEntry>, Box<dyn std::error::Error>>;

    async fn find_all(&self, capability: &str)
        -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>>;

    async fn find_by_id(&self, agent_id: &str)
        -> Result<Option<DiscoveryEntry>, Box<dyn std::error::Error>>;

    // ── interaction ────────────────────────────────────────────────────────
    async fn call(
        &self,
        agent_id:   &str,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>>;

    async fn call_capability(
        &self,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>>;

    async fn call_entry(
        &self,
        entry:      &DiscoveryEntry,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>>;
}

// ── AgentClient — HTTP implementation ────────────────────────────────────────

pub struct AgentClient {
    discovery: Arc<dyn IAgentDiscovery>,
    options:   AgentClientOptions,
}

impl AgentClient {
    pub fn new(discovery: Arc<dyn IAgentDiscovery>, options: AgentClientOptions) -> Self {
        Self { discovery, options }
    }
}

#[async_trait]
impl IAgentClient for AgentClient {
    // ── lookup ──────────────────────────────────────────────────────────────

    async fn find(
        &self, capability: &str,
    ) -> Result<Option<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let entries = self.discovery.query(capability).await?;
        Ok(entries.into_iter()
            .find(|e| e.health.status == "healthy")
            .or_else(|| self.discovery.query(capability).await.ok()?.into_iter().next()))
    }

    async fn find_all(
        &self, capability: &str,
    ) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let entries = self.discovery.query(capability).await?;
        let healthy: Vec<_> = entries.iter().filter(|e| e.health.status == "healthy").cloned().collect();
        Ok(if healthy.is_empty() { entries } else { healthy })
    }

    async fn find_by_id(
        &self, agent_id: &str,
    ) -> Result<Option<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let all = self.discovery.list_all().await?;
        Ok(all.into_iter().find(|e| e.agent_id == agent_id))
    }

    // ── interaction ─────────────────────────────────────────────────────────

    async fn call(
        &self,
        agent_id:   &str,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>> {
        let entry = self.find_by_id(agent_id).await?
            .ok_or_else(|| format!("Agent not found: {agent_id}"))?;
        self.call_entry(&entry, capability, payload, options).await
    }

    async fn call_capability(
        &self,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>> {
        let entry = self.find(capability).await?
            .ok_or_else(|| format!("No healthy agent for capability: {capability}"))?;
        self.call_entry(&entry, capability, payload, options).await
    }

    async fn call_entry(
        &self,
        entry:      &DiscoveryEntry,
        capability: &str,
        payload:    serde_json::Value,
        options:    CallOptions,
    ) -> Result<AgentResponse, Box<dyn std::error::Error>> {
        let caller_id  = options.caller_id.unwrap_or_else(|| self.options.caller_id.clone());
        let timeout_ms = options.timeout_ms.unwrap_or(self.options.timeout_ms);
        let req = AgentRequest {
            request_id:  Uuid::new_v4().to_string(),
            from:        caller_id,
            capability:  capability.to_string(),
            payload:     serde_json::to_string(&payload)?,
            signature:   None,
            timestamp:   Some(chrono::Utc::now().timestamp_millis()),
            session_key: None,
            payment:     None,
        };
        let url     = endpoint_url(entry);
        let client  = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()?;
        let resp: AgentResponse = client
            .post(&url)
            .json(&req)
            .send().await?
            .json().await?;
        Ok(resp)
    }
}

fn endpoint_url(entry: &DiscoveryEntry) -> String {
    let scheme = if entry.network.tls { "https" } else { &entry.network.protocol };
    let scheme = if ["http", "https"].contains(&scheme) { scheme } else { "http" };
    format!("{}://{}:{}/invoke", scheme, entry.network.host, entry.network.port)
}
