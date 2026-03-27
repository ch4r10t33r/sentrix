// NOTE: The SSE streaming endpoint (`/invoke/stream`) added below requires
// `futures-util = "0.3"` in Cargo.toml (under [dependencies]).
// Add the following line to your Cargo.toml.tpl or Cargo.toml:
//   futures-util = { version = "0.3", default-features = false, features = ["alloc"] }

use axum::{
    extract::State,
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::{self, Stream};
use serde_json::Value;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;
use tokio::signal;
use crate::agent::IAgent;
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use crate::discovery::DiscoveryEntry;
use serde::Deserialize;
use serde_json::json;

// ── State ─────────────────────────────────────────────────────────────────────

struct ServerState<A> {
    agent: A,
    started_at: Instant,
}

// ── Gossip request body ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GossipMessage {
    from: String,
    message: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /invoke — deserialise AgentRequest, call agent.handle_request(), return AgentResponse.
/// If the agent requires payment and none is provided, respond with HTTP 402.
async fn invoke_handler<A>(
    State(state): State<Arc<ServerState<A>>>,
    Json(request): Json<AgentRequest>,
) -> (StatusCode, Json<Value>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    // x402 payment gate
    if request.payment.is_none() && state.agent.requires_payment() {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({ "error": "payment required", "code": "402" })),
        );
    }

    let response: AgentResponse = state.agent.handle_request(request).await;
    (StatusCode::OK, Json(serde_json::to_value(response).unwrap_or_default()))
}

/// POST /invoke/stream — SSE streaming endpoint.
///
/// Accepts the same [`AgentRequest`] body as `/invoke`, calls
/// `agent.handle_request()`, and returns the result as a Server-Sent Events
/// stream.
///
/// ## Response format
///
/// ```text
/// data: {"requestId":"…","status":"success","result":{…}}\n\n
/// event: done\ndata: {}\n\n
/// ```
///
/// The stream currently sends two events:
/// 1. A `data:` event containing the full JSON-encoded [`AgentResponse`].
/// 2. A terminal `event: done` frame with an empty data payload.
///
/// ## True token-by-token streaming
///
/// Because `IAgent::handle_request` returns a single completed `AgentResponse`,
/// this endpoint simulates streaming by wrapping that response in an SSE stream.
/// Agents that need real incremental / token-by-token streaming should implement
/// an additional `async fn stream_request(&self, request: AgentRequest)
/// -> impl Stream<Item = AgentResponse>` method on `IAgent` (future work).
///
/// ## Payment gating
///
/// The same x402 payment check applied to `/invoke` is applied here. Requests
/// without a payment proof for priced capabilities receive HTTP 402 before any
/// SSE connection is established.
async fn invoke_stream_handler<A>(
    State(state): State<Arc<ServerState<A>>>,
    Json(request): Json<AgentRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<Value>)>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    // x402 payment gate — identical to /invoke.
    if request.payment.is_none() && state.agent.requires_payment() {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({ "error": "payment required", "code": "402" })),
        ));
    }

    // Execute the capability to completion.
    let response: AgentResponse = state.agent.handle_request(request).await;

    // Serialise the response for the first SSE data frame.
    let response_json = serde_json::to_string(&response).unwrap_or_else(|e| {
        json!({ "status": "error", "error_message": e.to_string() }).to_string()
    });

    // Build a two-event SSE stream:
    //   1. data: <full AgentResponse JSON>
    //   2. event: done / data: {}
    let events: Vec<Result<Event, Infallible>> = vec![
        Ok(Event::default().data(response_json)),
        Ok(Event::default().event("done").data("{}")),
    ];

    Ok(Sse::new(stream::iter(events)))
}

/// POST /gossip — accept { "from": "...", "message": "..." }, log it, return { "ok": true }.
async fn gossip_handler<A>(
    State(_state): State<Arc<ServerState<A>>>,
    Json(body): Json<GossipMessage>,
) -> (StatusCode, Json<Value>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    println!("[gossip] from={} message={}", body.from, body.message);
    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// GET /health — return { "status": "healthy", "uptime_seconds": <elapsed> }.
async fn health_handler<A>(
    State(state): State<Arc<ServerState<A>>>,
) -> (StatusCode, Json<Value>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let uptime = state.started_at.elapsed().as_secs();
    (StatusCode::OK, Json(json!({ "status": "healthy", "uptime_seconds": uptime })))
}

/// GET /anr — return the agent's DiscoveryEntry as JSON.
async fn anr_handler<A>(
    State(state): State<Arc<ServerState<A>>>,
) -> (StatusCode, Json<DiscoveryEntry>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let entry: DiscoveryEntry = state.agent.get_anr();
    (StatusCode::OK, Json(entry))
}

/// GET /capabilities — return { "capabilities": [...] }.
async fn capabilities_handler<A>(
    State(state): State<Arc<ServerState<A>>>,
) -> (StatusCode, Json<Value>)
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let caps: Vec<String> = state.agent.get_capabilities();
    (StatusCode::OK, Json(json!({ "capabilities": caps })))
}

// ── Shutdown signal ───────────────────────────────────────────────────────────

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            println!("[borgkit] Received SIGINT — shutting down");
        }
        _ = terminate => {
            println!("[borgkit] Received SIGTERM — shutting down");
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Start the Borgkit HTTP server and serve the given agent on the specified port.
/// Blocks until a SIGINT or SIGTERM is received, then shuts down gracefully.
pub async fn serve<A>(agent: A, port: u16) -> Result<(), Box<dyn std::error::Error>>
where
    A: IAgent + Clone + Send + Sync + 'static,
{
    let state = Arc::new(ServerState {
        agent,
        started_at: Instant::now(),
    });

    let app = Router::new()
        .route("/invoke",        post(invoke_handler::<A>))
        .route("/invoke/stream", post(invoke_stream_handler::<A>))
        .route("/gossip",        post(gossip_handler::<A>))
        .route("/health",        get(health_handler::<A>))
        .route("/anr",           get(anr_handler::<A>))
        .route("/capabilities",  get(capabilities_handler::<A>))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[borgkit] Listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
