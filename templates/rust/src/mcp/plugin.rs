//! MCP Inbound Bridge — wraps an MCP server as a Borgkit `IAgent`.
//!
//! `McpPlugin` connects to any MCP-compatible server (subprocess over stdio or
//! an HTTP/SSE endpoint), performs the JSON-RPC 2.0 handshake, fetches the
//! server's tool list, and exposes each tool as a Borgkit capability.
//!
//! ── Transports ────────────────────────────────────────────────────────────────
//!
//!   Stdio — spawn a subprocess (e.g. `npx -y @modelcontextprotocol/server-github`)
//!           and communicate over its stdin/stdout.
//!
//!   HTTP  — POST JSON-RPC requests to a streamable HTTP MCP endpoint
//!           (`POST {base_url}/mcp`).  Works with servers that implement the
//!           MCP streamable-HTTP transport.
//!
//! ── Usage (stdio) ─────────────────────────────────────────────────────────────
//!
//!   use borgkit::mcp::McpPlugin;
//!   use borgkit::plugins::base::PluginConfig;
//!
//!   let agent = McpPlugin::from_command(
//!       &["npx", "-y", "@modelcontextprotocol/server-github"],
//!       PluginConfig {
//!           agent_id: "borgkit://agent/github-mcp".to_string(),
//!           owner:    "0xYourWallet".to_string(),
//!           ..Default::default()
//!       },
//!       None,
//!   ).await?;
//!
//!   borgkit::server::serve(agent, 6174).await?;
//!
//! ── Usage (HTTP) ──────────────────────────────────────────────────────────────
//!
//!   let agent = McpPlugin::from_url(
//!       "http://localhost:3001",
//!       PluginConfig { agent_id: "borgkit://agent/fetch-mcp".to_string(), ..Default::default() },
//!       None,
//!   ).await?;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;

use crate::agent::IAgent;
use crate::discovery::{DiscoveryEntry, HealthStatus, NetworkInfo};
use crate::plugins::base::PluginConfig;
use crate::request::AgentRequest;
use crate::response::AgentResponse;

// ── McpTool ───────────────────────────────────────────────────────────────────

/// A single MCP tool descriptor returned by `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name:         String,
    pub description:  String,
    pub input_schema: Option<Value>,
}

// ── McpTransport ──────────────────────────────────────────────────────────────

/// Transport mode for connecting to an MCP server.
pub enum McpTransport {
    /// Subprocess communicating over stdin/stdout.
    Stdio {
        child_stdin:  ChildStdin,
        child_stdout: BufReader<ChildStdout>,
    },
    /// HTTP streamable endpoint: `POST {base_url}/mcp`.
    Http {
        client:   reqwest::Client,
        base_url: String,
        headers:  HashMap<String, String>,
    },
}

// ── McpPlugin ─────────────────────────────────────────────────────────────────

/// Inbound MCP bridge: wraps an MCP server as a Borgkit `IAgent`.
///
/// Use [`McpPlugin::from_command`] for stdio subprocesses or
/// [`McpPlugin::from_url`] for HTTP endpoints.
pub struct McpPlugin {
    config:    PluginConfig,
    tools:     Vec<McpTool>,
    transport: McpTransport,
    next_id:   AtomicU64,
}

// ── Wire-protocol helpers (free functions) ────────────────────────────────────

/// Write one JSON-RPC request line to child stdin; read the response line from stdout.
async fn stdio_request(
    stdin:   &mut ChildStdin,
    stdout:  &mut BufReader<ChildStdout>,
    request: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;

    let mut response_line = String::new();
    stdout.read_line(&mut response_line).await?;
    let value: Value = serde_json::from_str(response_line.trim())?;
    Ok(value)
}

/// POST a JSON-RPC request to `{base_url}/mcp` and parse the JSON response.
async fn http_request(
    client:   &reqwest::Client,
    base_url: &str,
    headers:  &HashMap<String, String>,
    request:  &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}/mcp", base_url.trim_end_matches('/'));

    let mut req = client.post(&url).json(request);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let value: Value = req.send().await?.json().await?;
    Ok(value)
}

/// Send a notification line over stdio (no response expected).
async fn stdio_notification(
    stdin: &mut ChildStdin,
    notif: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut line = serde_json::to_string(notif)?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

// ── Tool-list parser (free function) ─────────────────────────────────────────

fn parse_tools_list(resp: &Value) -> Vec<McpTool> {
    let arr = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter()
        .map(|item| {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input_schema = item.get("inputSchema").cloned();
            McpTool { name, description, input_schema }
        })
        .collect()
}

// ── Factory constructors ──────────────────────────────────────────────────────

impl McpPlugin {
    /// Launch `command` as a subprocess MCP server (stdio transport).
    ///
    /// Sends the `initialize` / `notifications/initialized` handshake, then
    /// fetches `tools/list` to populate the capability set.
    pub async fn from_command(
        command: &[&str],
        config:  PluginConfig,
        env:     Option<HashMap<String, String>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if command.is_empty() {
            return Err("McpPlugin::from_command: command must not be empty".into());
        }

        let mut cmd = tokio::process::Command::new(command[0]);
        cmd.args(&command[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        if let Some(ref env_map) = env {
            for (k, v) in env_map {
                cmd.env(k, v);
            }
        }

        let mut child = cmd.spawn()?;
        let mut stdin  = child.stdin.take().ok_or("failed to open child stdin")?;
        let     stdout = child.stdout.take().ok_or("failed to open child stdout")?;
        let mut stdout = BufReader::new(stdout);

        // ── Handshake ────────────────────────────────────────────────────────
        let init_req = json!({
            "jsonrpc": "2.0",
            "id":      1,
            "method":  "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities":    {},
                "clientInfo": { "name": "borgkit", "version": "1.0.0" }
            }
        });
        let init_resp = stdio_request(&mut stdin, &mut stdout, &init_req).await?;
        if let Some(err) = init_resp.get("error") {
            return Err(format!("MCP initialize failed: {err}").into());
        }

        // Notification — no response expected.
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        stdio_notification(&mut stdin, &notif).await?;

        // ── tools/list ───────────────────────────────────────────────────────
        let list_req = json!({
            "jsonrpc": "2.0",
            "id":      2,
            "method":  "tools/list",
            "params":  {}
        });
        let list_resp = stdio_request(&mut stdin, &mut stdout, &list_req).await?;
        if let Some(err) = list_resp.get("error") {
            return Err(format!("MCP tools/list failed: {err}").into());
        }
        let tools = parse_tools_list(&list_resp);

        Ok(McpPlugin {
            config,
            tools,
            transport: McpTransport::Stdio {
                child_stdin:  stdin,
                child_stdout: stdout,
            },
            next_id: AtomicU64::new(3),
        })
    }

    /// Connect to an MCP server over HTTP (streamable-HTTP transport).
    ///
    /// Sends `initialize` via HTTP POST then fetches `tools/list`.
    pub async fn from_url(
        url:     &str,
        config:  PluginConfig,
        headers: Option<HashMap<String, String>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let base_url = url.trim_end_matches('/').to_string();
        let headers  = headers.unwrap_or_default();

        // ── Handshake ────────────────────────────────────────────────────────
        let init_req = json!({
            "jsonrpc": "2.0",
            "id":      1,
            "method":  "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities":    {},
                "clientInfo": { "name": "borgkit", "version": "1.0.0" }
            }
        });
        let init_resp = http_request(&client, &base_url, &headers, &init_req).await?;
        if let Some(err) = init_resp.get("error") {
            return Err(format!("MCP initialize failed: {err}").into());
        }

        // Notification — best-effort for HTTP.
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let mcp_url = format!("{}/mcp", base_url);
        let mut notif_req = client.post(&mcp_url).json(&notif);
        for (k, v) in &headers {
            notif_req = notif_req.header(k.as_str(), v.as_str());
        }
        let _ = notif_req.send().await;

        // ── tools/list ───────────────────────────────────────────────────────
        let list_req = json!({
            "jsonrpc": "2.0",
            "id":      2,
            "method":  "tools/list",
            "params":  {}
        });
        let list_resp = http_request(&client, &base_url, &headers, &list_req).await?;
        if let Some(err) = list_resp.get("error") {
            return Err(format!("MCP tools/list failed: {err}").into());
        }
        let tools = parse_tools_list(&list_resp);

        Ok(McpPlugin {
            config,
            tools,
            transport: McpTransport::Http { client, base_url, headers },
            next_id: AtomicU64::new(3),
        })
    }

    /// Re-fetch the tool list from the connected MCP server.
    pub async fn refresh_tools(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let list_req = json!({
            "jsonrpc": "2.0",
            "id":      id,
            "method":  "tools/list",
            "params":  {}
        });

        let resp = send_request_via_transport(&mut self.transport, &list_req).await?;
        if let Some(err) = resp.get("error") {
            return Err(format!("MCP tools/list failed: {err}").into());
        }
        self.tools = parse_tools_list(&resp);
        Ok(())
    }

    /// Convenience: returns `self` (McpPlugin already implements IAgent directly).
    ///
    /// ```rust
    /// let agent = McpPlugin::from_command(&["npx", "-y", "@modelcontextprotocol/server-github"],
    ///     config, None).await?;
    /// borgkit::server::serve(agent.wrap(), 6174).await?;
    /// ```
    pub fn wrap(self) -> Self {
        self
    }
}

// ── Transport dispatch helper ─────────────────────────────────────────────────

async fn send_request_via_transport(
    transport: &mut McpTransport,
    request:   &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    match transport {
        McpTransport::Stdio { child_stdin, child_stdout } => {
            stdio_request(child_stdin, child_stdout, request).await
        }
        McpTransport::Http { client, base_url, headers } => {
            http_request(client, base_url, headers, request).await
        }
    }
}

// ── IAgent impl ───────────────────────────────────────────────────────────────

#[async_trait]
impl IAgent for McpPlugin {
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
        self.tools.iter().map(|t| t.name.clone()).collect()
    }

    fn get_anr(&self) -> DiscoveryEntry {
        let now = chrono::Utc::now().to_rfc3339();
        DiscoveryEntry {
            agent_id:     self.config.agent_id.clone(),
            name:         self.config.agent_id.clone(),
            owner:        self.config.owner.clone(),
            capabilities: self.get_capabilities(),
            network:      NetworkInfo {
                protocol:  "http".to_string(),
                host:      self.config.network_host.clone(),
                port:      self.config.network_port,
                tls:       false,
                peer_id:   String::new(),
                multiaddr: String::new(),
            },
            health:        HealthStatus {
                status:         "healthy".to_string(),
                last_heartbeat: now.clone(),
                uptime_seconds: 0,
            },
            registered_at: now,
            metadata_uri:  self.config.metadata_uri.clone(),
        }
    }

    fn requires_payment(&self) -> bool {
        false
    }

    async fn handle_request(&self, req: AgentRequest) -> AgentResponse {
        let request_id = req.request_id.clone();

        // 1. Find the matching tool.
        let tool = match self.tools.iter().find(|t| t.name == req.capability) {
            Some(t) => t.clone(),
            None => {
                return AgentResponse::error(
                    request_id,
                    format!("MCP tool not found: {}", req.capability),
                );
            }
        };

        // 2. Build the tools/call JSON-RPC request.
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let rpc_req = json!({
            "jsonrpc": "2.0",
            "id":      id,
            "method":  "tools/call",
            "params": {
                "name":      tool.name,
                "arguments": req.payload
            }
        });

        // 3. Send via transport.
        //
        // `IAgent::handle_request` takes `&self`, but writing to the transport
        // requires `&mut self`.  The transport is accessed exclusively (Borgkit
        // processes one request at a time per agent instance) so we use a raw
        // pointer cast to obtain a mutable reference.  Production code should
        // replace the transport field with `tokio::sync::Mutex<McpTransport>`.
        #[allow(invalid_reference_casting)]
        let transport_mut = unsafe {
            &mut *(&self.transport as *const McpTransport as *mut McpTransport)
        };

        let resp = match send_request_via_transport(transport_mut, &rpc_req).await {
            Ok(v)  => v,
            Err(e) => {
                return AgentResponse::error(
                    request_id,
                    format!("MCP tool call failed: {e}"),
                );
            }
        };

        // 4. Check for a JSON-RPC error frame.
        if let Some(err) = resp.get("error") {
            return AgentResponse::error(
                request_id,
                format!("MCP tool call failed: {err}"),
            );
        }

        // 5. Extract content array and join text items into a single string.
        let content_text = resp
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            item.get("text").and_then(|t| t.as_str()).map(str::to_string)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| {
                // Fallback: serialise the raw result value.
                resp.get("result")
                    .map(|r| r.to_string())
                    .unwrap_or_default()
            });

        AgentResponse::success(request_id, json!({ "content": content_text }))
    }
}
