//! LlamaIndex → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a LlamaIndex server so it participates in the Borgkit mesh as a
//! standard `IAgent`.  LlamaIndex agents and query engines can be served via
//! `llamaindex serve` or by using `llama_index.server` (FastAPI); this plugin
//! bridges both the `/chat` and `/query` endpoints.
//!
//! ── API contract ──────────────────────────────────────────────────────────────
//!
//!   Chat endpoint (default):
//!     POST {base_url}/chat
//!       Body:     { "message": "...", "chat_history": [] }
//!       Response: { "response": "...", "source_nodes": [...] }
//!
//!   Query endpoint (set invoke_route = "/query"):
//!     POST {base_url}/query
//!       Body:     { "query": "..." }
//!       Response: { "response": "...", "source_nodes": [...] }
//!
//! ── Setup ─────────────────────────────────────────────────────────────────────
//!
//!   Start a LlamaIndex server with:
//!     llamaindex serve --host 0.0.0.0 --port 8080
//!
//!   Or programmatically:
//!     from llama_index.server import LlamaIndexServer
//!     server = LlamaIndexServer(query_engine=my_engine)
//!     server.run()
//!
//! ── Usage ─────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::llamaindex::{LlamaIndexPlugin, LlamaIndexService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = LlamaIndexService {
//!       base_url:     "http://localhost:8080".to_string(),
//!       invoke_route: "/chat".to_string(),
//!       capabilities: vec![("search".to_string(), "Search the knowledge base".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = LlamaIndexPlugin::with_timeout(60);
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/llamaindex-rag".to_string(),
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

/// Configuration for a LlamaIndex HTTP server endpoint.
pub struct LlamaIndexService {
    /// Base URL, e.g. `http://localhost:8080`.
    pub base_url: String,

    /// POST path for invocation (default: `"/chat"`).
    ///
    /// Use `"/query"` to target the query engine endpoint instead.
    pub invoke_route: String,

    /// Optional Bearer token sent as `Authorization: Bearer <api_key>`.
    pub api_key: Option<String>,

    /// Explicit capability list: `(name, description)` pairs.
    ///
    /// When empty a single `"chat"` capability is synthesised automatically.
    pub capabilities: Vec<(String, String)>,
}

impl Default for LlamaIndexService {
    fn default() -> Self {
        Self {
            base_url:     "http://localhost:8080".to_string(),
            invoke_route: "/chat".to_string(),
            api_key:      None,
            capabilities: vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for LlamaIndex servers.
pub struct LlamaIndexPlugin {
    client: reqwest::Client,
}

impl LlamaIndexPlugin {
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

impl Default for LlamaIndexPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<LlamaIndexService> for LlamaIndexPlugin {
    fn extract_capabilities(&self, service: &LlamaIndexService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:           "chat".to_string(),
                description:    "Chat with or query the LlamaIndex agent".to_string(),
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

    /// Build the LlamaIndex request body from an `AgentRequest`.
    ///
    /// For the `/chat` route the body is `{ "message": ..., "chat_history": [] }`.
    /// For the `/query` route the body is `{ "query": ... }`.
    /// The route is not available here so the sentinel field `"__borgkit_content"`
    /// is stored; final body assembly happens in `invoke_native`.
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

        Ok(json!({ "__borgkit_content": content }))
    }

    /// Extract the `response` field from the LlamaIndex response JSON.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        let content = output
            .get("response")
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            })
            .unwrap_or_else(|| output.to_string());

        AgentResponse::success(
            request_id.to_string(),
            json!({ "content": content, "raw": output }),
        )
    }

    /// POST to `{base_url}{invoke_route}` with the appropriate body shape.
    ///
    /// Routes ending in `"/chat"` receive `{ "message": ..., "chat_history": [] }`;
    /// routes ending in `"/query"` receive `{ "query": ... }`.
    /// An optional Bearer token is added when `api_key` is configured.
    async fn invoke_native(
        &self,
        service: &LlamaIndexService,
        input:   Value,
    ) -> Result<Value, String> {
        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.invoke_route,
        );

        let content = input
            .get("__borgkit_content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Shape the body based on the configured route.
        let body = if service.invoke_route.ends_with("/query") {
            json!({ "query": content })
        } else {
            json!({ "message": content, "chat_history": [] })
        };

        let mut req = self.client.post(&url).json(&body);

        if let Some(ref key) = service.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        req.send()
            .await
            .map_err(|e| format!("LlamaIndex HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("LlamaIndex response parse error: {e}"))
    }
}
