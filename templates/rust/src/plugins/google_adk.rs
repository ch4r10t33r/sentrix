//! Google ADK → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps a running Google ADK web service (`adk web`) so it is discoverable
//! and callable on the Borgkit mesh as a standard `IAgent`.
//!
//! ── Google ADK HTTP API ───────────────────────────────────────────────────────
//!
//!   POST /run
//!     Body: {
//!       "app_name":   "my_app",
//!       "user_id":    "borgkit-user",
//!       "session_id": "<uuid>",
//!       "new_message": {
//!         "role":  "user",
//!         "parts": [{ "text": "..." }]
//!       }
//!     }
//!     Response: [Event, ...]   (array of ADK Event objects)
//!
//!   POST /apps/{app_name}/users/{user_id}/sessions
//!     Body: {}    → { "id": "<session_id>" }
//!
//! Start the ADK server with:
//!   adk web --port 8080 my_package/my_agent.py
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::google_adk::{GoogleADKPlugin, GoogleADKService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = GoogleADKService {
//!       base_url: "http://localhost:8080".to_string(),
//!       app_name: "my_agent".to_string(),
//!       capabilities: vec![("answer".to_string(), "Answer questions".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = GoogleADKPlugin::new();
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/gemini-support".to_string(),
//!       owner:        "0xYourWallet".to_string(),
//!       network_host: "localhost".to_string(),
//!       network_port: 6174,
//!       ..Default::default()
//!   });

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::plugins::base::{CapabilityDescriptor, BorgkitPlugin};
use crate::request::AgentRequest;
use crate::response::AgentResponse;

// ── Service config ────────────────────────────────────────────────────────────

/// Configuration for a Google ADK (`adk web`) HTTP endpoint.
pub struct GoogleADKService {
    /// Base URL of the ADK web server, e.g. `http://localhost:8080`.
    pub base_url: String,

    /// ADK application name (must match the `agent.py` agent name).
    pub app_name: String,

    /// User ID passed to every ADK session (default: `"borgkit-user"`).
    pub user_id: String,

    /// POST path for running the agent (default: `"/run"`).
    pub run_route: String,

    /// Explicit capabilities: `(name, description)` pairs.
    ///
    /// Empty = single `"invoke"` capability synthesised automatically.
    pub capabilities: Vec<(String, String)>,
}

impl Default for GoogleADKService {
    fn default() -> Self {
        Self {
            base_url:     "http://localhost:8080".to_string(),
            app_name:     "agent".to_string(),
            user_id:      "borgkit-user".to_string(),
            run_route:    "/run".to_string(),
            capabilities: vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for Google ADK.
pub struct GoogleADKPlugin {
    client: reqwest::Client,
}

impl GoogleADKPlugin {
    pub fn new() -> Self { Self::with_timeout(120) }

    pub fn with_timeout(secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for GoogleADKPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<GoogleADKService> for GoogleADKPlugin {
    fn extract_capabilities(&self, service: &GoogleADKService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:          "invoke".to_string(),
                description:   "Invoke the Google ADK agent".to_string(),
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

    /// Build the ADK `/run` request body from an `AgentRequest`.
    ///
    /// A fresh session ID is generated per call so that each Borgkit request
    /// is independently stateless.
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

        let session_id = Uuid::new_v4().to_string();

        Ok(json!({
            "app_name":   Value::Null,   // filled in invoke_native from service config
            "user_id":    Value::Null,   // filled in invoke_native
            "session_id": session_id,
            "new_message": {
                "role":  "user",
                "parts": [{ "text": message }]
            },
            "__borgkit_request_id__": request.request_id,
            "__borgkit_capability__": request.capability,
        }))
    }

    /// Extract readable text from ADK Event array.
    ///
    /// ADK returns an array of Event objects; we collect all text parts from
    /// `event.content.parts[*].text` and join them.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        let content = extract_adk_text(&output);
        AgentResponse::success(
            request_id.to_string(),
            json!({ "content": content, "raw": output }),
        )
    }

    /// POST to `{base_url}/run`, injecting `app_name` and `user_id` from service config.
    async fn invoke_native(
        &self,
        service: &GoogleADKService,
        mut input: Value,
    ) -> Result<Value, String> {
        // Fill in the placeholders set by translate_request
        input["app_name"] = json!(service.app_name);
        input["user_id"]  = json!(service.user_id);
        // Remove internal Borgkit tracking keys before sending
        let obj = input.as_object_mut().ok_or("invalid request body")?;
        obj.remove("__borgkit_request_id__");
        obj.remove("__borgkit_capability__");

        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.run_route,
        );

        self.client
            .post(&url)
            .json(&input)
            .send()
            .await
            .map_err(|e| format!("Google ADK HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("Google ADK response parse error: {e}"))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collect all `event.content.parts[*].text` from an ADK event array.
fn extract_adk_text(events: &Value) -> String {
    let arr = match events.as_array() {
        Some(a) => a,
        None    => return events.to_string(),
    };

    let mut parts: Vec<String> = Vec::new();
    for event in arr {
        if let Some(content) = event.get("content") {
            if let Some(ps) = content.get("parts").and_then(|p| p.as_array()) {
                for part in ps {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        if !text.is_empty() {
                            parts.push(text.to_string());
                        }
                    }
                }
            }
        }
    }

    if parts.is_empty() { events.to_string() } else { parts.join("\n") }
}
