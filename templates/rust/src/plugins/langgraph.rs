//! LangGraph → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a LangServe / LangGraph Platform endpoint so it participates in the
//! Borgkit mesh as a standard `IAgent`.
//!
//! ── LangServe API contract ────────────────────────────────────────────────────
//!
//!   POST {base_url}/invoke
//!     Body:     { "input": { "messages": [...] }, "config": { "recursion_limit": 25 } }
//!     Response: { "output": { "messages": [...] } }
//!
//!   POST {base_url}/stream   (optional — returns newline-delimited JSON events)
//!
//! Start a LangServe endpoint with:
//!   from langserve import add_routes
//!   add_routes(app, my_graph, path="/")
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::langgraph::{LangGraphPlugin, LangGraphService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = LangGraphService {
//!       base_url:    "http://localhost:8000".to_string(),
//!       capabilities: vec![("research".to_string(), "Research a topic".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = LangGraphPlugin::with_timeout(60);
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/researcher".to_string(),
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

/// Configuration for a LangServe / LangGraph Platform HTTP endpoint.
pub struct LangGraphService {
    /// Base URL, e.g. `http://localhost:8000`.
    pub base_url: String,

    /// POST path for single invocation (default: `"/invoke"`).
    pub invoke_route: String,

    /// LangGraph recursion limit forwarded in every request (default: `25`).
    pub recursion_limit: u32,

    /// Explicit capability list: `(name, description)` pairs.
    ///
    /// When empty a single `"invoke"` capability is synthesised automatically.
    pub capabilities: Vec<(String, String)>,

    /// The key inside the graph state that carries messages (default: `"messages"`).
    pub input_key: String,

    /// The key inside `output` that carries the reply messages (default: `"messages"`).
    pub output_key: String,
}

impl Default for LangGraphService {
    fn default() -> Self {
        Self {
            base_url:        "http://localhost:8000".to_string(),
            invoke_route:    "/invoke".to_string(),
            recursion_limit: 25,
            capabilities:    vec![],
            input_key:       "messages".to_string(),
            output_key:      "messages".to_string(),
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for LangGraph / LangServe.
pub struct LangGraphPlugin {
    client: reqwest::Client,
}

impl LangGraphPlugin {
    /// Create with a default 60-second timeout.
    pub fn new() -> Self {
        Self::with_timeout(60)
    }

    /// Create with a custom per-request timeout.
    pub fn with_timeout(secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for LangGraphPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<LangGraphService> for LangGraphPlugin {
    fn extract_capabilities(&self, service: &LangGraphService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:          "invoke".to_string(),
                description:   "Invoke the LangGraph agent".to_string(),
                input_schema:  None,
                output_schema: None,
                price_per_call: None,
            }];
        }
        service.capabilities.iter().map(|(name, desc)| CapabilityDescriptor {
            name:          name.clone(),
            description:   desc.clone(),
            input_schema:  None,
            output_schema: None,
            price_per_call: None,
        }).collect()
    }

    /// Build the LangServe-compatible JSON body from an `AgentRequest`.
    ///
    /// The payload field `"message"`, `"input"`, or `"query"` (first found) becomes
    /// the human message content; otherwise the whole payload is stringified.
    fn translate_request(&self, request: &AgentRequest) -> Result<Value, String> {
        let content = request.payload.get("message")
            .or_else(|| request.payload.get("input"))
            .or_else(|| request.payload.get("query"))
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            })
            .unwrap_or_else(|| request.payload.to_string());

        Ok(json!({
            "input": {
                "messages": [{ "role": "human", "content": content }]
            },
            "config": { "recursion_limit": 25 },
        }))
    }

    /// Extract the last AI message from the LangServe response envelope.
    ///
    /// LangServe wraps every response as `{ "output": { "messages": [...] } }`.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        let content = output.get("output")
            .and_then(|o| extract_last_ai_message(o))
            .unwrap_or_else(|| output.to_string());

        AgentResponse::success(
            request_id.to_string(),
            json!({ "content": content, "raw": output }),
        )
    }

    /// POST the translated body to `{base_url}{invoke_route}` and return the JSON response.
    async fn invoke_native(
        &self,
        service: &LangGraphService,
        input:   Value,
    ) -> Result<Value, String> {
        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.invoke_route,
        );

        self.client
            .post(&url)
            .json(&input)
            .send()
            .await
            .map_err(|e| format!("LangGraph HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("LangGraph response parse error: {e}"))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Walk `output.messages` in reverse and return the first AI / assistant message.
fn extract_last_ai_message(output: &Value) -> Option<String> {
    let messages = output.get("messages")?.as_array()?;
    for msg in messages.iter().rev() {
        let type_ = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let role  = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if type_ == "ai" || type_ == "AIMessage" || role == "assistant" {
            let content = msg.get("content")?;
            return Some(if content.is_string() {
                content.as_str().unwrap_or("").to_string()
            } else {
                content.to_string()
            });
        }
    }
    // Fall back to last message if no AI tag found
    messages.last().and_then(|m| {
        m.get("content").map(|c| if c.is_string() {
            c.as_str().unwrap_or("").to_string()
        } else {
            c.to_string()
        })
    })
}
