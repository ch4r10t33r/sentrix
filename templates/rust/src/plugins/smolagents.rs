//! smolagents → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a deployed HuggingFace smolagents server so it participates in the
//! Borgkit mesh as a standard `IAgent`.  smolagents can be served in two modes:
//!
//!   • Gradio UI  — `GradioUI(agent).launch()` exposes a Gradio `/run/predict`
//!                  endpoint (default; `use_gradio: true`).
//!   • Custom API — a thin FastAPI wrapper around the agent exposes a `/run`
//!                  endpoint (`use_gradio: false`).
//!
//! ── API contract ──────────────────────────────────────────────────────────────
//!
//!   Gradio predict endpoint (use_gradio: true, default):
//!     POST {base_url}/run/predict
//!       Body:     { "data": ["task text"] }
//!       Response: { "data": ["output text"], "duration": 1.2 }
//!
//!   Custom FastAPI endpoint (use_gradio: false):
//!     POST {base_url}/run
//!       Body:     { "task": "...", "kwargs": {} }
//!       Response: { "output": "...", "steps": [...] }
//!
//! ── Setup ─────────────────────────────────────────────────────────────────────
//!
//!   Gradio (default, port 7860):
//!     from smolagents import GradioUI, CodeAgent, HfApiModel
//!     agent = CodeAgent(tools=[], model=HfApiModel())
//!     GradioUI(agent).launch()
//!
//!   Custom FastAPI wrapper:
//!     from fastapi import FastAPI
//!     from smolagents import CodeAgent, HfApiModel
//!     app = FastAPI()
//!     agent = CodeAgent(tools=[], model=HfApiModel())
//!     @app.post("/run")
//!     def run(payload: dict): return {"output": agent.run(payload["task"])}
//!
//! ── Usage ─────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::smolagents::{SmolagentsPlugin, SmolagentsService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   // Gradio mode (default)
//!   let service = SmolagentsService {
//!       base_url:     "http://localhost:7860".to_string(),
//!       capabilities: vec![("run".to_string(), "Run a task with the smolagent".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = SmolagentsPlugin::with_timeout(120);
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/smolagent".to_string(),
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

/// Configuration for a smolagents HTTP server endpoint.
pub struct SmolagentsService {
    /// Base URL, e.g. `http://localhost:7860`.
    pub base_url: String,

    /// POST path for invocation (default: `"/run/predict"`).
    ///
    /// Override to `"/run"` when `use_gradio` is `false`.
    pub invoke_route: String,

    /// Use the Gradio predict API format (default: `true`).
    ///
    /// When `true`:  body = `{ "data": [content] }`, reply = `data[0]`.
    /// When `false`: body = `{ "task": content, "kwargs": {} }`, reply = `output`.
    pub use_gradio: bool,

    /// Optional Bearer token sent as `Authorization: Bearer <api_key>`.
    pub api_key: Option<String>,

    /// Explicit capability list: `(name, description)` pairs.
    ///
    /// When empty a single `"run"` capability is synthesised automatically.
    pub capabilities: Vec<(String, String)>,
}

impl Default for SmolagentsService {
    fn default() -> Self {
        Self {
            base_url:     "http://localhost:7860".to_string(),
            invoke_route: "/run/predict".to_string(),
            use_gradio:   true,
            api_key:      None,
            capabilities: vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for smolagents servers (Gradio or custom FastAPI).
pub struct SmolagentsPlugin {
    client: reqwest::Client,
}

impl SmolagentsPlugin {
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

impl Default for SmolagentsPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<SmolagentsService> for SmolagentsPlugin {
    fn extract_capabilities(&self, service: &SmolagentsService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:           "run".to_string(),
                description:    "Run a task with the smolagent".to_string(),
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

    /// Build the smolagents request body from an `AgentRequest`.
    ///
    /// The payload field `"message"`, `"input"`, `"task"`, or `"query"` (first found)
    /// becomes the task content; otherwise the whole payload is stringified.
    /// The `use_gradio` flag is not available here; a sentinel field is stored and
    /// the final body is assembled in `invoke_native`.
    fn translate_request(&self, request: &AgentRequest) -> Result<Value, String> {
        let content = request.payload.get("message")
            .or_else(|| request.payload.get("input"))
            .or_else(|| request.payload.get("task"))
            .or_else(|| request.payload.get("query"))
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            })
            .unwrap_or_else(|| request.payload.to_string());

        Ok(json!({ "__borgkit_content": content }))
    }

    /// Extract the agent output from the smolagents response.
    ///
    /// Gradio mode:     reads `data[0]`.
    /// Custom API mode: reads `output`.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        // Try Gradio format first (`data` array), then custom API format (`output`).
        let content = output
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            })
            .or_else(|| {
                output.get("output").map(|v| if v.is_string() {
                    v.as_str().unwrap_or("").to_string()
                } else {
                    v.to_string()
                })
            })
            .unwrap_or_else(|| output.to_string());

        AgentResponse::success(
            request_id.to_string(),
            json!({ "content": content, "raw": output }),
        )
    }

    /// POST to `{base_url}{invoke_route}` with the appropriate body shape.
    ///
    /// Gradio mode (`use_gradio: true`):  body = `{ "data": [content] }`.
    /// Custom API mode (`use_gradio: false`): body = `{ "task": content, "kwargs": {} }`.
    /// An optional Bearer token is added when `api_key` is configured.
    async fn invoke_native(
        &self,
        service: &SmolagentsService,
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

        let body = if service.use_gradio {
            json!({ "data": [content] })
        } else {
            json!({ "task": content, "kwargs": {} })
        };

        let mut req = self.client.post(&url).json(&body);

        if let Some(ref key) = service.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        req.send()
            .await
            .map_err(|e| format!("smolagents HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("smolagents response parse error: {e}"))
    }
}
