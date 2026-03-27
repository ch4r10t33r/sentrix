use crate::agent::IAgent;
use crate::discovery::{DiscoveryEntry, HealthStatus, LocalDiscovery, NetworkInfo};
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use async_trait::async_trait;
use serde_json::json;

pub struct ExampleAgent {
    pub discovery: LocalDiscovery,
}

impl ExampleAgent {
    pub fn new() -> Self {
        Self { discovery: LocalDiscovery::default() }
    }

    fn anr(&self) -> DiscoveryEntry {
        let host = std::env::var("BORGKIT_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("BORGKIT_PORT")
            .ok().and_then(|p| p.parse().ok()).unwrap_or(6174u16);
        let peer_id = self.get_peer_id();
        let multiaddr = peer_id.as_ref().map(|pid|
            format!("/ip4/{}/tcp/{}/p2p/{}", host, port, pid)
        ).unwrap_or_default();

        DiscoveryEntry {
            agent_id:      self.agent_id().into(),
            name:          "ExampleAgent".into(),
            owner:         self.owner().into(),
            capabilities:  self.get_capabilities(),
            network:       NetworkInfo {
                protocol: if peer_id.is_some() { "libp2p".into() } else { "http".into() },
                host,
                port,
                tls:      false,
                peer_id:   peer_id.unwrap_or_default(),
                multiaddr,
            },
            health:        HealthStatus {
                status:         "healthy".into(),
                last_heartbeat: chrono::Utc::now().to_rfc3339(),
                uptime_seconds: 0,
            },
            registered_at: chrono::Utc::now().to_rfc3339(),
            metadata_uri:  Some("ipfs://QmYourMetadataHashHere".into()),
        }
    }
}

#[async_trait]
impl IAgent for ExampleAgent {
    fn agent_id(&self) -> &str { "borgkit://agent/example" }
    fn owner(&self)    -> &str { "0xYourWalletAddress" }

    fn metadata_uri(&self) -> Option<&str> {
        Some("ipfs://QmYourMetadataHashHere")
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["echo".into(), "ping".into()]
    }

    async fn handle_request(&self, req: AgentRequest) -> AgentResponse {
        if !self.check_permission(&req.from, &req.capability).await {
            return AgentResponse::error(req.request_id, "Permission denied".into());
        }

        match req.capability.as_str() {
            "echo" => AgentResponse::success(req.request_id, json!({ "echo": req.payload })),
            "ping" => AgentResponse::success(req.request_id, json!({
                "pong":    true,
                "agentId": self.agent_id(),
                "version": "0.1.0",
            })),
            _ => AgentResponse::error(req.request_id, format!("Unknown capability: {}", req.capability)),
        }
    }

    /// Return this agent's fully-populated ANR (Agent Network Record).
    ///
    /// Override this in production to reflect your real host, port, and TLS settings.
    fn get_anr(&self) -> DiscoveryEntry {
        self.anr()
    }

    /// Return the libp2p PeerId for this agent.
    ///
    /// To enable: generate a secp256k1 key, persist it (e.g. ~/.config/myagent/key),
    /// load the 32-byte raw private key, and call `peer_id_from_anr_key(&raw_key)`.
    ///
    /// Example:
    /// ```rust
    /// use crate::anr::peer_id_from_anr_key;
    ///
    /// fn get_peer_id(&self) -> Option<String> {
    ///     let raw = std::fs::read("/path/to/key").ok()?;
    ///     peer_id_from_anr_key(raw.as_slice()).ok()
    /// }
    /// ```
    fn get_peer_id(&self) -> Option<String> {
        None // no signing key configured in this example
    }

    async fn register_discovery(&self) -> Result<(), Box<dyn std::error::Error>> {
        use crate::discovery::IAgentDiscovery;
        self.discovery.register(self.anr()).await
    }
}
