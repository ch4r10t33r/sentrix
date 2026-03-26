use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
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
            println!("[sentrix] Received SIGINT — shutting down");
        }
        _ = terminate => {
            println!("[sentrix] Received SIGTERM — shutting down");
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Start the Sentrix HTTP server and serve the given agent on the specified port.
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
        .route("/invoke",       post(invoke_handler::<A>))
        .route("/gossip",       post(gossip_handler::<A>))
        .route("/health",       get(health_handler::<A>))
        .route("/anr",          get(anr_handler::<A>))
        .route("/capabilities", get(capabilities_handler::<A>))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[sentrix] Listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
