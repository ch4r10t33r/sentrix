//! MCP Outbound Bridge — exposes any Borgkit `IAgent` as an MCP server.
//!
//! Receives MCP JSON-RPC 2.0 messages from an MCP client (e.g. Claude Desktop,
//! Cursor, Cline) and dispatches them to the wrapped `IAgent`.  Each Borgkit
//! capability is advertised as an MCP tool.
//!
//! ── Transports ────────────────────────────────────────────────────────────────
//!
//!   Stdio     — read JSON-RPC frames from stdin, write responses to stdout.
//!               Used by MCP hosts that spawn the agent as a subprocess.
//!
//!   SSE       — HTTP server with:
//!                 GET  /sse        → Server-Sent Events stream per client
//!                 POST /messages   → client sends JSON-RPC requests here
//!                 GET  /health     → liveness probe
//!
//!   Http      — stateless HTTP server:
//!                 POST /mcp        → one request → one JSON response
//!                 GET  /health     → liveness probe
//!
//! ── Protocol direction ────────────────────────────────────────────────────────
//!
//!   MCP client (Claude Desktop / Cursor / Cline)
//!       ──────────────────────────────────────────▶  serve_as_mcp (this module)
//!       ◀──────────────────────────────────────────       IAgent
//!
//! ── Claude Desktop (stdio) config ─────────────────────────────────────────────
//!
//!   {
//!     "mcpServers": {
//!       "my-agent": {
//!         "command": "/path/to/my-agent-binary",
//!         "args":    []
//!       }
//!     }
//!   }
//!
//! ── Claude Desktop (SSE) config ───────────────────────────────────────────────
//!
//!   {
//!     "mcpServers": {
//!       "my-agent": {
//!         "url": "http://localhost:3000/sse"
//!       }
//!     }
//!   }
//!
//! ── Usage (stdio) ─────────────────────────────────────────────────────────────
//!
//!   use borgkit::mcp::{serve_as_mcp, ServeMcpOptions, Transport};
//!
//!   serve_as_mcp(my_agent, ServeMcpOptions::default()).await?;
//!
//! ── Usage (HTTP streamable) ───────────────────────────────────────────────────
//!
//!   serve_as_mcp(my_agent, ServeMcpOptions {
//!       transport: Transport::Http,
//!       port:      3000,
//!       ..Default::default()
//!   }).await?;
//!
//! ── Usage (SSE) ───────────────────────────────────────────────────────────────
//!
//!   serve_as_mcp(my_agent, ServeMcpOptions {
//!       transport: Transport::Sse,
//!       port:      3000,
//!       ..Default::default()
//!   }).await?;

// NOTE: This file requires `futures-util = "0.3"` in Cargo.toml.
// Add to [dependencies]:
//   futures-util = { version = "0.3", default-features = false, features = ["alloc"] }

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::IAgent;
use crate::request::AgentRequest;
use crate::response::AgentResponse;

// ── Transport ─────────────────────────────────────────────────────────────────

/// Wire transport for the outbound MCP server.
#[derive(Debug, Clone, Copy)]
pub enum Transport {
    /// Read/write newline-delimited JSON on stdin/stdout.
    Stdio,
    /// HTTP server with SSE stream (`GET /sse`) and POST endpoint (`POST /messages`).
    Sse,
    /// Stateless streamable HTTP: `POST /mcp` returns the JSON-RPC response directly.
    Http,
}

// ── ServeMcpOptions ───────────────────────────────────────────────────────────

/// Options for [`serve_as_mcp`].
pub struct ServeMcpOptions {
    /// Override the `serverInfo.name` field in the `initialize` response.
    /// Defaults to `"borgkit-agent"`.
    pub name:      Option<String>,
    /// Wire transport to use (default: [`Transport::Stdio`]).
    pub transport: Transport,
    /// Bind host for SSE / HTTP transports (default: `"0.0.0.0"`).
    pub host:      String,
    /// Bind port for SSE / HTTP transports (default: `3000`).
    pub port:      u16,
}

impl Default for ServeMcpOptions {
    fn default() -> Self {
        Self {
            name:      None,
            transport: Transport::Stdio,
            host:      "0.0.0.0".to_string(),
            port:      3000,
        }
    }
}

// ── MCP method dispatch (shared logic) ───────────────────────────────────────

/// Build the `tools/list` result array for the given agent.
fn build_tools_list<A: IAgent>(agent: &A) -> Value {
    let tools: Vec<Value> = agent
        .get_capabilities()
        .into_iter()
        .filter(|cap| !cap.starts_with("__"))
        .map(|cap| {
            json!({
                "name":        cap,
                "description": format!("Borgkit capability: {cap}"),
                "inputSchema": {
                    "type":       "object",
                    "properties": {
                        "payload": { "type": "object" }
                    }
                }
            })
        })
        .collect();

    json!({ "tools": tools })
}

/// Handle a single JSON-RPC request object and return the JSON-RPC response.
///
/// Returns `None` for notifications (no `"id"` field — no response expected).
async fn handle_rpc<A>(agent: &A, server_name: &str, msg: &Value) -> Option<Value>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id     = msg.get("id").cloned();

    // JSON-RPC notifications carry no "id" — do not send a response.
    let id = match id {
        Some(v) if !v.is_null() => v,
        _                        => return None,
    };

    let result: Value = match method {
        // ── initialize ───────────────────────────────────────────────────────
        "initialize" => {
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities":    { "tools": {} },
                "serverInfo": {
                    "name":    server_name,
                    "version": "0.1.0"
                }
            })
        }

        // ── tools/list ───────────────────────────────────────────────────────
        "tools/list" => build_tools_list(agent),

        // ── tools/call ───────────────────────────────────────────────────────
        "tools/call" => {
            let params    = msg.get("params").cloned().unwrap_or_default();
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let request_id = Uuid::new_v4().to_string();
            let agent_req  = AgentRequest {
                request_id:  request_id.clone(),
                from:        "mcp-client".to_string(),
                capability:  tool_name,
                payload:     arguments,
                signature:   None,
                timestamp:   None,
                session_key: None,
                payment:     None,
            };

            let agent_resp: AgentResponse = agent.handle_request(agent_req).await;

            let text = if agent_resp.status == "success" {
                // Prefer the "content" string inside the result, otherwise serialise.
                agent_resp
                    .result
                    .as_ref()
                    .and_then(|r| r.get("content"))
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        agent_resp
                            .result
                            .as_ref()
                            .map(|r| r.to_string())
                            .unwrap_or_default()
                    })
            } else {
                agent_resp
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "tool call failed".to_string())
            };

            json!({
                "content": [{ "type": "text", "text": text }]
            })
        }

        // ── notifications/initialized (belt-and-suspenders: also handled via id check) ──
        "notifications/initialized" => return None,

        // ── unknown method ───────────────────────────────────────────────────
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id":      id,
                "error": {
                    "code":    -32601,
                    "message": "Method not found"
                }
            }));
        }
    };

    Some(json!({
        "jsonrpc": "2.0",
        "id":      id,
        "result":  result
    }))
}

// ── serve_as_mcp ─────────────────────────────────────────────────────────────

/// Expose any `IAgent` as an MCP server.
///
/// Blocks until the process exits (stdio) or until the OS terminates the
/// process (SSE / HTTP).
pub async fn serve_as_mcp<A>(
    agent:   A,
    options: ServeMcpOptions,
) -> Result<(), Box<dyn std::error::Error>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let server_name = options
        .name
        .clone()
        .unwrap_or_else(|| "borgkit-agent".to_string());

    match options.transport {
        Transport::Stdio => serve_stdio(agent, server_name).await,
        Transport::Sse   => serve_sse(agent, server_name, &options.host, options.port).await,
        Transport::Http  => serve_http(agent, server_name, &options.host, options.port).await,
    }
}

// ── Stdio transport ───────────────────────────────────────────────────────────

async fn serve_stdio<A>(
    agent:       A,
    server_name: String,
) -> Result<(), Box<dyn std::error::Error>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let stdin  = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut writer = tokio::io::BufWriter::new(stdout);

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF — client disconnected.
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v)  => v,
            Err(e) => {
                // JSON-RPC parse error (-32700).
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id":      null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                let mut out = serde_json::to_string(&err_resp)?;
                out.push('\n');
                writer.write_all(out.as_bytes()).await?;
                writer.flush().await?;
                continue;
            }
        };

        if let Some(response) = handle_rpc(&agent, &server_name, &msg).await {
            let mut out = serde_json::to_string(&response)?;
            out.push('\n');
            writer.write_all(out.as_bytes()).await?;
            writer.flush().await?;
        }
        // Notifications produce None — nothing to write.
    }

    Ok(())
}

// ── SSE transport ─────────────────────────────────────────────────────────────

/// Per-session SSE sender map: session_id → mpsc sender.
type SessionMap = Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<String>>>>;

/// Shared state for the SSE transport axum app.
#[derive(Clone)]
struct SseState<A: IAgent + Clone + Send + Sync + 'static> {
    agent:       A,
    server_name: String,
    sessions:    SessionMap,
}

/// Query params for `POST /messages`.
#[derive(Debug, Deserialize)]
struct SessionIdQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// `GET /sse` — open a new SSE stream for the client.
///
/// Sends an `endpoint` event immediately with the session-specific POST URL,
/// then relays all JSON-RPC response frames produced by `POST /messages`.
async fn sse_connect_handler<A>(
    State(state): State<SseState<A>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx)   = tokio::sync::mpsc::channel::<String>(64);

    state.sessions.lock().await.insert(session_id.clone(), tx);

    // The initial event tells the client where to POST its JSON-RPC requests.
    let endpoint_data = format!("/messages?sessionId={session_id}");

    // Wrap the mpsc receiver into a `Stream` using `futures_util::stream::unfold`.
    let rx = tokio::sync::Mutex::new(rx);
    let msg_stream = futures_util::stream::unfold(rx, |mut guard| async move {
        let msg = guard.lock().await.recv().await?;
        Some((Ok::<Event, Infallible>(Event::default().data(msg)), guard))
    });

    // Prepend the endpoint event using `futures_util::stream::once` + `chain`.
    let first_event = futures_util::stream::once(std::future::ready(
        Ok::<Event, Infallible>(
            Event::default()
                .event("endpoint")
                .data(endpoint_data),
        ),
    ));

    Sse::new(futures_util::stream::StreamExt::chain(first_event, msg_stream))
}

/// `POST /messages?sessionId=<id>` — receive a JSON-RPC request from the client
/// and push the response down the matching SSE stream.
async fn sse_message_handler<A>(
    State(state):  State<SseState<A>>,
    Query(query):  Query<SessionIdQuery>,
    Json(body):    Json<Value>,
) -> impl IntoResponse
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    if let Some(response) = handle_rpc(&state.agent, &state.server_name, &body).await {
        let json_str = serde_json::to_string(&response).unwrap_or_default();
        let sessions = state.sessions.lock().await;
        if let Some(tx) = sessions.get(&query.session_id) {
            let _ = tx.send(json_str).await;
        }
    }
    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// `GET /health` — liveness probe (SSE transport).
async fn sse_health_handler() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "healthy" })))
}

async fn serve_sse<A>(
    agent:       A,
    server_name: String,
    host:        &str,
    port:        u16,
) -> Result<(), Box<dyn std::error::Error>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    let state = SseState { agent, server_name, sessions };

    let app = Router::new()
        .route("/sse",      get(sse_connect_handler::<A>))
        .route("/messages", post(sse_message_handler::<A>))
        .route("/health",   get(sse_health_handler))
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[borgkit-mcp] SSE server listening on http://{addr}");
    println!("[borgkit-mcp] Connect Claude Desktop to: http://{addr}/sse");

    axum::serve(listener, app).await?;
    Ok(())
}

// ── HTTP Streamable transport ─────────────────────────────────────────────────

/// Shared state for the HTTP streamable axum app.
#[derive(Clone)]
struct HttpState<A: IAgent + Clone + Send + Sync + 'static> {
    agent:       A,
    server_name: String,
}

/// `POST /mcp` — receive a JSON-RPC request; return the JSON-RPC response directly.
async fn http_mcp_handler<A>(
    State(state): State<HttpState<A>>,
    Json(body):   Json<Value>,
) -> (StatusCode, Json<Value>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    match handle_rpc(&state.agent, &state.server_name, &body).await {
        Some(response) => (StatusCode::OK, Json(response)),
        None           => (StatusCode::OK, Json(json!({}))),
    }
}

/// `GET /health` — liveness probe (HTTP transport).
async fn http_health_handler() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "healthy" })))
}

async fn serve_http<A>(
    agent:       A,
    server_name: String,
    host:        &str,
    port:        u16,
) -> Result<(), Box<dyn std::error::Error>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let state = HttpState { agent, server_name };

    let app = Router::new()
        .route("/mcp",    post(http_mcp_handler::<A>))
        .route("/health", get(http_health_handler))
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[borgkit-mcp] HTTP server listening on http://{addr}");
    println!("[borgkit-mcp] MCP streamable endpoint: http://{addr}/mcp");

    axum::serve(listener, app).await?;
    Ok(())
}
