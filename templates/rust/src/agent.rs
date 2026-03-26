use crate::request::AgentRequest;
use crate::response::AgentResponse;
use crate::discovery::DiscoveryEntry;
use async_trait::async_trait;

/// ERC-8004 compliant agent trait.
/// Every Sentrix agent must implement this.
#[async_trait]
pub trait IAgent: Send + Sync {
    // ── ERC-8004 Identity ──────────────────────────────────────────────────
    fn agent_id(&self) -> &str;
    fn owner(&self) -> &str;
    fn metadata_uri(&self) -> Option<&str> { None }

    // ── Capabilities ───────────────────────────────────────────────────────
    fn get_capabilities(&self) -> Vec<String>;

    // ── Request handling ───────────────────────────────────────────────────
    async fn handle_request(&self, request: AgentRequest) -> AgentResponse;

    async fn pre_process(&self, _request: &AgentRequest) -> Result<(), String> {
        Ok(()) // override for auth / rate-limiting
    }

    async fn post_process(&self, _response: &AgentResponse) -> Result<(), String> {
        Ok(()) // override for audit logging / billing
    }

    // ── Discovery (optional) ───────────────────────────────────────────────
    async fn register_discovery(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn unregister_discovery(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    // ── Payment gating (x402) ──────────────────────────────────────────────

    /// Return `true` if this agent requires an x402 payment on every `/invoke`
    /// call.  The default is `false` (open access).  Override and return `true`
    /// when `required_payment` is non-empty.
    fn requires_payment(&self) -> bool {
        false
    }

    // ── Permissions (optional) ─────────────────────────────────────────────
    async fn check_permission(&self, _caller: &str, _capability: &str) -> bool {
        true // open by default; override for production
    }

    // ── ANR / Identity exposure ────────────────────────────────────────────

    /// Return the full ANR (Agent Network Record) for this agent.
    ///
    /// The ANR is the authoritative self-description of the agent on the mesh.
    /// Override this to return a complete `DiscoveryEntry` populated from your
    /// agent's fields. The default implementation returns a minimal placeholder.
    fn get_anr(&self) -> DiscoveryEntry {
        let now = chrono::Utc::now().to_rfc3339();
        DiscoveryEntry {
            agent_id:      self.agent_id().to_string(),
            name:          self.agent_id().to_string(),
            owner:         self.owner().to_string(),
            capabilities:  self.get_capabilities(),
            network: crate::discovery::NetworkInfo {
                protocol: "http".to_string(),
                host:     "localhost".to_string(),
                port:     8080,
                tls:      false,
            },
            health: crate::discovery::HealthStatus {
                status:         "healthy".to_string(),
                last_heartbeat: now.clone(),
                uptime_seconds: 0,
            },
            registered_at: now,
            metadata_uri:  self.metadata_uri().map(str::to_string),
        }
    }

    /// Return the libp2p PeerId derived from this agent's secp256k1 ANR key.
    ///
    /// Returns `None` for anonymous agents (no signing key). Override and
    /// call `peer_id_from_anr_key(&raw_private_key)` to return a real PeerId.
    fn get_peer_id(&self) -> Option<String> {
        None
    }

    // ── Signing (optional) ─────────────────────────────────────────────────
    async fn sign_message(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Err("sign_message not implemented".into())
    }
}
