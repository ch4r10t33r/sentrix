use async_trait::async_trait;
use serde_json::Value;
use crate::agent::IAgent;
use crate::discovery::{DiscoveryEntry, HealthStatus, NetworkInfo};
use crate::request::AgentRequest;
use crate::response::AgentResponse;

// ── CapabilityDescriptor ──────────────────────────────────────────────────────

/// Metadata about a capability exposed by a wrapped agent.
pub struct CapabilityDescriptor {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the expected input shape.
    pub input_schema: Option<Value>,
    /// JSON Schema describing the output shape.
    pub output_schema: Option<Value>,
    /// Human-readable price per call, e.g. `"0.001 USDC"`. `None` = free.
    pub price_per_call: Option<String>,
}

// ── PluginConfig ──────────────────────────────────────────────────────────────

/// Configuration passed to a plugin at wrap time.
pub struct PluginConfig {
    pub agent_id: String,
    pub owner: String,
    /// Explicit capability list. `None` means auto-extract from the native agent.
    pub capabilities: Option<Vec<CapabilityDescriptor>>,
    pub metadata_uri: Option<String>,
    /// Hostname where this agent will be reachable. Defaults to `"localhost"`.
    pub network_host: String,
    /// Port where this agent will be reachable. Defaults to `6174`.
    pub network_port: u16,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            agent_id:     String::new(),
            owner:        String::new(),
            capabilities: None,
            metadata_uri: None,
            network_host: "localhost".to_string(),
            network_port: 6174,
        }
    }
}

// ── BorgkitPlugin ─────────────────────────────────────────────────────────────

/// A plugin wraps a foreign-framework agent and makes it `IAgent`-compatible.
///
/// Implement this trait to bridge any third-party agent framework (LangChain,
/// Eliza, CrewAI, etc.) into the Borgkit mesh.
#[async_trait]
pub trait BorgkitPlugin<TAgent>: Send + Sync {
    /// Extract capability descriptors from the native agent.
    fn extract_capabilities(&self, agent: &TAgent) -> Vec<CapabilityDescriptor>;

    /// Translate a Borgkit `AgentRequest` into the native framework's input type
    /// (as a `serde_json::Value`).
    fn translate_request(&self, request: &AgentRequest) -> Result<Value, String>;

    /// Translate the native agent's output back into an `AgentResponse`.
    fn translate_response(&self, request_id: &str, native_output: Value) -> AgentResponse;

    /// Invoke the native agent with the translated input and return its raw output.
    async fn invoke_native(
        &self,
        agent: &TAgent,
        input: Value,
    ) -> Result<Value, String>;

    /// Convenience method: wrap the native agent into a `WrappedAgent`.
    ///
    /// Capabilities are resolved once at wrap time:
    /// - if `config.capabilities` is `Some(list)`, that list is used as-is.
    /// - if `config.capabilities` is `None`, `extract_capabilities` is called.
    fn wrap(self, agent: TAgent, config: PluginConfig) -> WrappedAgent<TAgent, Self>
    where
        Self: Sized,
    {
        WrappedAgent::new(agent, self, config)
    }
}

// ── WrappedAgent ──────────────────────────────────────────────────────────────

/// An `IAgent`-compatible wrapper around any native agent + plugin pair.
pub struct WrappedAgent<TAgent, P: BorgkitPlugin<TAgent>> {
    agent:        TAgent,
    plugin:       P,
    config:       PluginConfig,
    capabilities: Vec<CapabilityDescriptor>,
    started_at:   std::time::Instant,
}

impl<TAgent, P: BorgkitPlugin<TAgent>> WrappedAgent<TAgent, P> {
    /// Create a new `WrappedAgent`.  Capabilities are resolved here:
    /// explicit list from `config.capabilities`, or auto-extracted via the plugin.
    pub fn new(agent: TAgent, plugin: P, mut config: PluginConfig) -> Self {
        let capabilities = config
            .capabilities
            .take()
            .unwrap_or_else(|| plugin.extract_capabilities(&agent));
        Self {
            capabilities,
            agent,
            plugin,
            config,
            started_at: std::time::Instant::now(),
        }
    }
}

// ── IAgent impl ───────────────────────────────────────────────────────────────

#[async_trait]
impl<TAgent, P> IAgent for WrappedAgent<TAgent, P>
where
    TAgent: Send + Sync,
    P:      BorgkitPlugin<TAgent> + Send + Sync,
{
    fn agent_id(&self) -> &str {
        &self.config.agent_id
    }

    fn owner(&self) -> &str {
        &self.config.owner
    }

    fn metadata_uri(&self) -> Option<&str> {
        self.config.metadata_uri.as_deref()
    }

    fn get_capabilities(&self) -> Vec<String> {
        self.capabilities.iter().map(|c| c.name.clone()).collect()
    }

    async fn handle_request(&self, request: AgentRequest) -> AgentResponse {
        let request_id = request.request_id.clone();

        // 1. Translate Borgkit request → native input
        let native_input = match self.plugin.translate_request(&request) {
            Ok(v)    => v,
            Err(msg) => return AgentResponse::error(request_id, msg),
        };

        // 2. Invoke the native agent
        let native_output = match self.plugin.invoke_native(&self.agent, native_input).await {
            Ok(v)    => v,
            Err(msg) => return AgentResponse::error(request_id, msg),
        };

        // 3. Translate native output → AgentResponse
        self.plugin.translate_response(&request_id, native_output)
    }

    fn get_anr(&self) -> DiscoveryEntry {
        let now = chrono::Utc::now().to_rfc3339();
        let uptime = self.started_at.elapsed().as_secs();
        DiscoveryEntry {
            agent_id:      self.config.agent_id.clone(),
            name:          self.config.agent_id.clone(),
            owner:         self.config.owner.clone(),
            capabilities:  self.get_capabilities(),
            network: NetworkInfo {
                protocol: "http".to_string(),
                host:     self.config.network_host.clone(),
                port:     self.config.network_port,
                tls:      false,
            },
            health: HealthStatus {
                status:         "healthy".to_string(),
                last_heartbeat: now.clone(),
                uptime_seconds: uptime,
            },
            registered_at: now,
            metadata_uri:  self.config.metadata_uri.clone(),
        }
    }

    /// Returns `true` if any registered capability has a `price_per_call` set.
    fn requires_payment(&self) -> bool {
        self.capabilities.iter().any(|c| c.price_per_call.is_some())
    }
}
