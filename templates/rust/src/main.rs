//! Borgkit Rust template — dev runner
//!
//! Demonstrates:
//!   1. Building and registering an agent.
//!   2. Printing the agent's ANR (Agent Network Record).
//!   3. Creating an AgentClient that shares the same in-memory discovery layer.
//!   4. Calling a capability by name (discover-and-call in one step).
//!   5. Calling a specific agent by ID.

mod agent;
mod anr;
mod client;
mod discovery;
mod example_agent;
mod request;
mod response;

use std::sync::Arc;

use crate::agent::IAgent;
use crate::client::{AgentClient, AgentClientOptions, CallOptions};
use crate::example_agent::ExampleAgent;
use crate::request::AgentRequest;
use serde_json::json;

#[tokio::main]
async fn main() {
    // ── 1. Build the agent ───────────────────────────────────────────────────

    let agent = ExampleAgent::new();

    // ── 2. Register with the local discovery layer ────────────────────────────
    //
    //  In production, replace LocalDiscovery with HttpDiscovery or GossipDiscovery
    //  and point it at your registry URL.

    agent.register_discovery().await.expect("Discovery registration failed");

    // ── 3. Inspect the ANR ───────────────────────────────────────────────────
    //
    //  The ANR is the agent's authoritative self-description on the mesh.
    //  It contains the agent_id, owner, capabilities, network endpoint, and
    //  optional metadata URI.  Other agents use it to look up and call you.

    let anr = agent.get_anr();
    println!("\n  Agent ID   : {}", anr.agent_id);
    println!("  Owner      : {}", anr.owner);
    println!("  Endpoint   : {}://{}:{}", anr.network.protocol, anr.network.host, anr.network.port);
    println!("  Capabilities: {:?}", anr.capabilities);
    if let Some(uri) = &anr.metadata_uri {
        println!("  Metadata   : {}", uri);
    }
    if let Some(peer_id) = agent.get_peer_id() {
        println!("  Peer ID    : {}", peer_id);
    }
    println!();

    // ── 4. Local smoke test ───────────────────────────────────────────────────
    //
    //  Call the agent directly (in-process), without going through HTTP.

    let req = AgentRequest {
        request_id:  "test-local-001".into(),
        from:        "0xCaller".into(),
        capability:  "ping".into(),
        payload:     json!({}),
        signature:   None,
        timestamp:   None,
        session_key: None,
        payment:     None,
    };

    let resp = agent.handle_request(req).await;
    println!("  [local] ping response: {:?}", resp);

    // ── 5. AgentClient — discover-and-call ────────────────────────────────────
    //
    //  AgentClient wraps the discovery layer and dispatches requests over HTTP.
    //  Here we share the same LocalDiscovery by cloning it — LocalDiscovery
    //  uses Arc<Mutex<...>> internally so both ends see the same registry.
    //
    //  In a real setup the client and the agent run in separate processes;
    //  they both point at the same HttpDiscovery / GossipDiscovery registry.

    let shared_discovery = Arc::new(agent.discovery.clone());
    let client = AgentClient::new(
        shared_discovery.clone(),
        AgentClientOptions {
            caller_id:  "borgkit://agent/caller".into(),
            timeout_ms: 5_000,
        },
    );

    // 5a. Discover the best healthy agent for "echo" and call it
    match client.call_capability("echo", json!({ "message": "hello" }), CallOptions::default()).await {
        Ok(resp)  => println!("  [client] call_capability(echo): {:?}", resp),
        Err(err)  => println!("  [client] call_capability failed (expected in local mode): {}", err),
    }

    // 5b. Call a specific agent by ID
    match client.call(
        "borgkit://agent/example",
        "ping",
        json!({}),
        CallOptions::default(),
    ).await {
        Ok(resp)  => println!("  [client] call(ping): {:?}", resp),
        Err(err)  => println!("  [client] call failed (expected in local mode): {}", err),
    }

    // 5c. Lookup only — no invocation
    match client.find("ping").await {
        Ok(Some(entry)) => println!("  [client] find(ping) → {} @ {}:{}", entry.agent_id, entry.network.host, entry.network.port),
        Ok(None)        => println!("  [client] find(ping) → no agent registered"),
        Err(err)        => println!("  [client] find failed: {}", err),
    }

    println!("\n  AgentClient.call_capability() dispatches over HTTP to the discovered endpoint.");
    println!("  In this dev runner both agent and client share the same in-memory LocalDiscovery.\n");
}

// To run as a standalone HTTP server:
// use crate::server::serve;
// serve(agent, 6174).await.expect("Server failed");
