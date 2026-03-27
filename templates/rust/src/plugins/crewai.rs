//! CrewAI → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a running CrewAI HTTP service so its crews are discoverable and
//! callable on the Borgkit mesh as a standard `IAgent`.
//!
//! CrewAI is Python-native; this plugin communicates over HTTP so Rust agents
//! can invoke CrewAI crews without embedding a Python interpreter.
//!
//! ── Expected service endpoints ────────────────────────────────────────────────
//!
//!   GET  /capabilities
//!     → [{ "name": "...", "description": "...", "parameters"?: {...} }]
//!
//!   POST /kickoff
//!     Body:     { "capability"?: "...", "task"?: "...", "inputs": {...} }
//!     Response: { "result": "...", "status": "success"|"error", "error"?: "..." }
//!
//! ── Serving CrewAI over HTTP ──────────────────────────────────────────────────
//!
//!   # serve_crew.py  (FastAPI wrapper — drop next to your crew)
//!   from fastapi import FastAPI
//!   from my_crew import my_crew
//!
//!   app = FastAPI()
//!
//!   @app.get("/capabilities")
//!   def caps():
//!       return [{"name": "kickoff", "description": "Run the crew on a task"}]
//!
//!   @app.post("/kickoff")
//!   async def kickoff(body: dict):
//!       result = my_crew.kickoff(inputs=body.get("inputs", {}))
//!       return {"result": str(result), "status": "success"}
//!
//!   # uvicorn serve_crew:app --port 8000
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::crewai::{CrewAIPlugin, CrewAIService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = CrewAIService {
//!       base_url: "http://localhost:8000".to_string(),
//!       ..Default::default()
//!   };
//!
//!   let plugin = CrewAIPlugin::new();
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/writer-crew".to_string(),
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

// ── Service config ────────────────────────────────────────────────────────────

/// Configuration for a CrewAI HTTP service endpoint.
pub struct CrewAIService {
    /// Base URL of the service, e.g. `http://localhost:8000`.
    pub base_url: String,

    /// POST path for crew execution (default: `"/kickoff"`).
    pub kickoff_route: String,

    /// GET path for capability discovery (default: `"/capabilities"`).
    pub capabilities_route: String,

    /// Optional Bearer token sent as `Authorization` header.
    pub api_key: Option<String>,

    /// Explicit capabilities: `(name, description)` pairs.
    ///
    /// When non-empty, `capabilities_route` is never called.
    /// When empty, a single `"invoke"` capability is synthesised at wrap time;
    /// call `CrewAIPlugin::fetch_capabilities` to populate from the service.
    pub capabilities: Vec<(String, String)>,
}

impl Default for CrewAIService {
    fn default() -> Self {
        Self {
            base_url:            "http://localhost:8000".to_string(),
            kickoff_route:       "/kickoff".to_string(),
            capabilities_route:  "/capabilities".to_string(),
            api_key:             None,
            capabilities:        vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for CrewAI.
pub struct CrewAIPlugin {
    client: reqwest::Client,
}

impl CrewAIPlugin {
    pub fn new() -> Self { Self::with_timeout(120) }

    pub fn with_timeout(secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Fetch capabilities from GET /capabilities and update `service.capabilities`.
    ///
    /// Call this after construction but before `wrap()` if you want the wrapped
    /// agent to advertise the full tool list in discovery.
    pub async fn fetch_capabilities(
        &self,
        service: &mut CrewAIService,
    ) -> Result<(), String> {
        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.capabilities_route,
        );

        let resp: Value = self.get_with_auth(&url, service.api_key.as_deref())
            .await
            .map_err(|e| format!("CrewAI capabilities fetch error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("CrewAI capabilities parse error: {e}"))?;

        if let Some(arr) = resp.as_array() {
            service.capabilities = arr.iter().filter_map(|c| {
                let name = c.get("name")?.as_str()?.to_string();
                let desc = c.get("description").and_then(|d| d.as_str())
                    .unwrap_or("").to_string();
                Some((name, desc))
            }).collect();
        }

        Ok(())
    }

    async fn get_with_auth(
        &self,
        url:     &str,
        api_key: Option<&str>,
    ) -> reqwest::Result<reqwest::Response> {
        let mut req = self.client.get(url);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }
        req.send().await
    }
}

impl Default for CrewAIPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<CrewAIService> for CrewAIPlugin {
    fn extract_capabilities(&self, service: &CrewAIService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:          "invoke".to_string(),
                description:   "Invoke the CrewAI crew".to_string(),
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

    /// Map a Borgkit `AgentRequest` into a CrewAI kickoff body.
    ///
    /// The `"task"`, `"query"`, or `"input"` payload key (first found) becomes the
    /// task description; otherwise the whole payload is forwarded as `inputs`.
    fn translate_request(&self, request: &AgentRequest) -> Result<Value, String> {
        let task = request.payload.get("task")
            .or_else(|| request.payload.get("query"))
            .or_else(|| request.payload.get("input"))
            .map(|v| if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            });

        Ok(json!({
            "capability": request.capability,
            "task":       task,
            "inputs":     request.payload,
        }))
    }

    /// Map a CrewAI service response back to an `AgentResponse`.
    ///
    /// Handles `{ "result": "...", "status": "..." }` and plain string bodies.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        if output.get("status").and_then(|s| s.as_str()) == Some("error") {
            let msg = output.get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return AgentResponse::error(request_id.to_string(), msg);
        }

        let content = output.get("result")
            .or_else(|| output.get("output"))
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

    /// POST the kickoff body to `{base_url}{kickoff_route}`.
    async fn invoke_native(
        &self,
        service: &CrewAIService,
        input:   Value,
    ) -> Result<Value, String> {
        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.kickoff_route,
        );

        let mut req = self.client.post(&url).json(&input);
        if let Some(key) = &service.api_key {
            req = req.bearer_auth(key);
        }

        req.send()
            .await
            .map_err(|e| format!("CrewAI HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("CrewAI response parse error: {e}"))
    }
}
