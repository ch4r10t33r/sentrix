//! OpenAI-compatible Chat Completions → Borgkit Plugin (Rust) — HTTP Bridge
//!
//! Wraps any OpenAI-compatible chat completions endpoint so it participates in
//! the Borgkit mesh as a standard `IAgent`.  Works with the official OpenAI API,
//! vLLM, Ollama (`/v1/chat/completions`), LocalAI, LM Studio, and any server that
//! implements the OpenAI chat completions contract.
//!
//! ── API contract ──────────────────────────────────────────────────────────────
//!
//!   POST {base_url}/v1/chat/completions
//!     Body:     { "model": "gpt-4o-mini",
//!                 "messages": [{ "role": "user", "content": "..." }],
//!                 "max_tokens": 1024 }
//!     Response: { "choices": [{ "message": { "role": "assistant",
//!                                             "content": "..." } }] }
//!
//!   Authorization: Bearer <api_key>   (omitted when api_key is None)
//!
//! ── Setup ─────────────────────────────────────────────────────────────────────
//!
//!   OpenAI:  set base_url = "https://api.openai.com" and api_key = Some("sk-...")
//!   Ollama:  set base_url = "http://localhost:11434"  (no api_key required)
//!   vLLM:    set base_url = "http://localhost:8000"   (no api_key required)
//!
//! ── Usage ─────────────────────────────────────────────────────────────────────
//!
//!   use borgkit::plugins::openai::{OpenAIPlugin, OpenAIService};
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let service = OpenAIService {
//!       base_url:      "https://api.openai.com".to_string(),
//!       model:         "gpt-4o-mini".to_string(),
//!       api_key:       Some("sk-...".to_string()),
//!       system_prompt: Some("You are a helpful assistant.".to_string()),
//!       capabilities:  vec![("chat".to_string(), "Answer questions via GPT".to_string())],
//!       ..Default::default()
//!   };
//!
//!   let plugin = OpenAIPlugin::with_timeout(30);
//!   let agent  = plugin.wrap(service, PluginConfig {
//!       agent_id:     "borgkit://agent/gpt".to_string(),
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

/// Configuration for an OpenAI-compatible chat completions HTTP endpoint.
pub struct OpenAIService {
    /// Base URL, e.g. `https://api.openai.com` or `http://localhost:11434`.
    pub base_url: String,

    /// Model name forwarded in every request (default: `"gpt-4o-mini"`).
    pub model: String,

    /// POST path for chat completions (default: `"/v1/chat/completions"`).
    pub invoke_route: String,

    /// Optional Bearer token sent as `Authorization: Bearer <api_key>`.
    ///
    /// When `None` the `Authorization` header is omitted entirely.
    pub api_key: Option<String>,

    /// Maximum tokens the model may generate per response (default: `1024`).
    pub max_tokens: u32,

    /// Optional system prompt prepended as a `{ "role": "system" }` message.
    ///
    /// When `None` the messages array contains only the user turn.
    pub system_prompt: Option<String>,

    /// Explicit capability list: `(name, description)` pairs.
    ///
    /// When empty a single `"chat"` capability is synthesised automatically.
    pub capabilities: Vec<(String, String)>,
}

impl Default for OpenAIService {
    fn default() -> Self {
        Self {
            base_url:      "https://api.openai.com".to_string(),
            model:         "gpt-4o-mini".to_string(),
            invoke_route:  "/v1/chat/completions".to_string(),
            api_key:       None,
            max_tokens:    1024,
            system_prompt: None,
            capabilities:  vec![],
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for any OpenAI-compatible chat completions API.
pub struct OpenAIPlugin {
    client: reqwest::Client,
}

impl OpenAIPlugin {
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

impl Default for OpenAIPlugin {
    fn default() -> Self { Self::new() }
}

// ── BorgkitPlugin impl ────────────────────────────────────────────────────────

#[async_trait]
impl BorgkitPlugin<OpenAIService> for OpenAIPlugin {
    fn extract_capabilities(&self, service: &OpenAIService) -> Vec<CapabilityDescriptor> {
        if service.capabilities.is_empty() {
            return vec![CapabilityDescriptor {
                name:           "chat".to_string(),
                description:    "Chat with an OpenAI-compatible language model".to_string(),
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

    /// Build the OpenAI-compatible JSON body from an `AgentRequest`.
    ///
    /// The payload field `"message"`, `"input"`, or `"query"` (first found) becomes
    /// the user message content; otherwise the whole payload is stringified.
    /// A system message is prepended when `system_prompt` is configured.
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

        // Build messages array; system prompt is injected at position 0 when set.
        // The actual service reference is not available here, so we produce the
        // user turn only — system prompt injection happens in invoke_native where
        // the service config is accessible.
        Ok(json!({ "__borgkit_content": content }))
    }

    /// Extract the assistant reply from `choices[0].message.content`.
    fn translate_response(&self, request_id: &str, output: Value) -> AgentResponse {
        let content = output
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|msg| msg.get("content"))
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

    /// POST the translated body to `{base_url}{invoke_route}`.
    ///
    /// The `Authorization: Bearer <api_key>` header is added when an API key is
    /// configured.  System prompt injection and final body assembly happen here
    /// because the service config is available at this call site.
    async fn invoke_native(
        &self,
        service: &OpenAIService,
        input:   Value,
    ) -> Result<Value, String> {
        let url = format!(
            "{}{}",
            service.base_url.trim_end_matches('/'),
            service.invoke_route,
        );

        // Recover the raw content string extracted in translate_request.
        let content = input
            .get("__borgkit_content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Assemble messages: optional system message followed by the user turn.
        let mut messages: Vec<Value> = Vec::new();
        if let Some(ref sys) = service.system_prompt {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        messages.push(json!({ "role": "user", "content": content }));

        let body = json!({
            "model":      service.model,
            "messages":   messages,
            "max_tokens": service.max_tokens,
        });

        let mut req = self.client.post(&url).json(&body);

        if let Some(ref key) = service.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        req.send()
            .await
            .map_err(|e| format!("OpenAI HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("OpenAI response parse error: {e}"))
    }
}
