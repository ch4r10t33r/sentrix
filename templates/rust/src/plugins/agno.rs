//! Agno → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a deployed Agno FastAPI server so it participates in the Borgkit mesh
//! as a standard `IAgent`.  Agno agents are served via `app.run()` which starts
//! a FastAPI server; this plugin bridges both the multi-agent route and the
//! simpler single-agent `/run` route.
//!
//! ── API contract ──────────────────────────────────────────────────────────────
//!
//!   Multi-agent route (agent_id is set):
//!     POST {base_url}/v1/agents/{agent_id}/runs
//!       Body:     { "message": "...", "stream": false }
//!       Response: { "content": "...", "run_id": "..." }
//!
//!   Single-agent route (agent_id is None):
//!     POST {base_url}/run
//!       Body:     { "message": "...", "stream": false }
//!       Response: { "content": "...", "messages": [...] }
//!
//!   Streaming (stream: true) — returns newline-delimited JSON events;
//!   this plugin always disables streaming and reads the full response.
//!
//! ── Setup ─────────────────────────────────────────────────────────────────────
//!
//!   Start your Agno agent server with:
//!     from agno.app import App
//!     app = App(agents=[my_agent])
//!     app.run()               # starts on http://localhost:7777 by default
//!
//! ── Usage ─────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::agno::{AgnoPlugin, AgnoService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = AgnoService {
//!       base_url:     "http://localhost:7777".to_string(),
//!       agent_id:     Some("my-research-agent".to_string()),
//!       capabilities: vec![("research".to_string(), "Research a topic".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = AgnoPlugin::with_timeout(120);
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/agno-researcher".to_string(),
//!       owner:        "0xYourWallet".to_string(),
//!       network_host: "localhost".to_string(),
//!       network_port: 6174,
//!       ..Default::default()
//!   });

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::plugins::base::{CapabilityDescriptor, BorgkitPlugin};
use crate::request::AgentRequest;
use crate::response::AgentResponse;

// ── Service config (the "agent" token) ───────────────────────────────────────

/// Configuration for an Agno FastAPI HTTP endpoint.
pub struct AgnoService {
    /// Base URL, e.g. `http://localhost:7777`.
    pub base_url: String,

    /// Optional Agno agent identifier.
    ///
    /// When set, requests are routed to `/v1/agents/{agent_id}/runs`.
    /// When `None`, requests are sent to `invoke_route` (`"/run"` by default).
    pub agent_id: Option<String>,

    /// Fallback POST path used when `agent_id` is `None` (default: `"/run"`).
    pub invoke_route: String,

    /// Optional Bearer token sent as `Authorization: Bearer <api_key>`.
    pub api_key: Option<String>,

    /// Whether to request a streaming response from the server (default: `false`).
    ///
    /// This plugin always reads the full JSON response regardless of this setting;
    /// it is forwarded in the request body for server-side stream control.
    pub stream: bool,

    /// Explicit capability list: `(name, description)` pairs.
    ///
    /// When empty a single `"run"` capability is synthesised automatically.
    pub capabilities: Vec<(String, String)>,
}

impl Default for AgnoService {
    fn default() -> Self {
        Self {
            base_url:     "http://localhost:7777".to_string(),
            agent_id:     None,
            invoke_route: "/run".to_string(),
            api_key:      None,
            stream:       false,
            capabilities: vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for Agno FastAPI servers.
pub struct AgnoPlugin {
    client: reqwest::Client,
}

impl AgnoPlugin {
    /// Create with a default 60-second timeout.
    pub fn new() -> Self {
        Self::with_timeout(60)
    }

    /// Create with a custom per-request timeout in seconds.
    pub fn with_timeout(secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for AgnoPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<AgnoService> for AgnoPlugin {
    fn extract_capabilities(&self, service: &AgnoService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:           "run".to_string(),
                description:    "Run the Agno agent with a message".to_string(),
                input_schema:   None,
                output_schema:  None,
                price_per_call: None,
            }];
        }
        service.capabilities.iter().map(|(name, desc)| CapabilityDescriptor {
            name:           name.clone(),
            description:    desc.clone(),
            input_schema:   None,
            output_schema:  None,
            price_per_call: None,
        }).collect()
    }

    /// Build the Agno request body from an `AgentRequest`.
    ///
    /// The payload field `"message"`, `"input"`, or `"query"` (first found) becomes
    /// the message content; otherwise the whole payload is stringified.
    /// Streaming is always disabled (`"stream": false`).
    fn translate_request(&self, request: &AgentRequest) -> Result<Value, String> {
        let message = request.payload.get("message")
            .or_else(|| request.payload.get("input"))
            .or_else(|| request.payload.get("query"))
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            })
            .unwrap_or_else(|| request.payload.to_string());

        Ok(json!({
            "message": message,
            "stream":  false,
        }))
    }

    /// Extract the reply from the Agno response.
    ///
    /// Tries `content` first; if absent, walks `messages` in reverse and returns
    /// the first assistant-role message content found.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        let content = output.get("content")
            .and_then(|v| if v.is_string() { v.as_str().map(|s| s.to_string()) } else { None })
            .or_else(|| {
                output.get("messages")
                    .and_then(|m| m.as_array())
                    .and_then(|msgs| {
                        msgs.iter().rev().find_map(|msg| {
                            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                            if role == "assistant" || role == "agent" {
                                msg.get("content").and_then(|c| {
                                    if c.is_string() {
                                        c.as_str().map(|s| s.to_string())
                                    } else {
                                        Some(c.to_string())
                                    }
                                })
                            } else {
                                None
                            }
                        })
                    })
            })
            .unwrap_or_else(|| output.to_string());

        AgentResponse::success(
            request_id.to_string(),
            json!({ "content": content, "raw": output }),
        )
    }

    /// POST the translated body to the appropriate Agno endpoint.
    ///
    /// When `agent_id` is set the route is `/v1/agents/{agent_id}/runs`;
    /// otherwise `invoke_route` is used.  An optional Bearer token is added
    /// when `api_key` is configured.
    async fn invoke_native(
        &self,
        service: &AgnoService,
        input:   Value,
    ) -> Result<Value, String> {
        let route = match &service.agent_id {
            Some(id) => format!("/v1/agents/{}/runs", id),
            None     => service.invoke_route.clone(),
        };

        let url = format!("{}{}", service.base_url.trim_end_matches('/'), route);

        let mut req = self.client.post(&url).json(&input);

        if let Some(ref key) = service.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        req.send()
            .await
            .map_err(|e| format!("Agno HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("Agno response parse error: {e}"))
    }
}
