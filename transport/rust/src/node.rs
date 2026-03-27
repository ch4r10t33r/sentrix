//! BorgkitNode — owns the libp2p Swarm and drives its event loop.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    gossipsub::{self, MessageId, PublishError},
    identify,
    noise,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
    Multiaddr, PeerId, StreamProtocol, Swarm,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::invoke::InvokeCodec;

pub const INVOKE_PROTO:  &str = "/borgkit/invoke/1.0.0";
pub const GOSSIP_TOPIC:  &str = "/borgkit/gossip/1.0.0";

// ── message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub from: String,
    pub capability: String,
    pub payload: serde_json::Value,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub status: String,
    pub result: Option<serde_json::Value>,
    #[serde(rename = "errorMessage", skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub timestamp: u64,
}

impl AgentResponse {
    pub fn error(request_id: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            status: "error".into(),
            result: None,
            error_message: Some(msg.into()),
            timestamp: unix_ms(),
        }
    }
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── behaviour ─────────────────────────────────────────────────────────────────

#[derive(NetworkBehaviour)]
pub struct BorgkitBehaviour {
    pub invoke:    request_response::Behaviour<InvokeCodec>,
    pub gossipsub: gossipsub::Behaviour,
    pub identify:  identify::Behaviour,
}

// ── node config ───────────────────────────────────────────────────────────────

pub struct BorgkitNodeConfig {
    pub listen_addrs: Vec<Multiaddr>,
}

impl Default for BorgkitNodeConfig {
    fn default() -> Self {
        Self {
            listen_addrs: vec!["/ip4/0.0.0.0/tcp/0".parse().unwrap()],
        }
    }
}

// ── commands sent to the swarm event loop ─────────────────────────────────────

enum SwarmCmd {
    Dial {
        addr:   Multiaddr,
        result: oneshot::Sender<Result<(), String>>,
    },
    Send {
        peer:    PeerId,
        request: AgentRequest,
        result:  oneshot::Sender<Result<AgentResponse, String>>,
    },
    Publish {
        data:   Vec<u8>,
        result: oneshot::Sender<Result<MessageId, String>>,
    },
    PeerInfo {
        result: oneshot::Sender<(PeerId, Vec<Multiaddr>)>,
    },
}

// ── BorgkitNode ───────────────────────────────────────────────────────────────

/// A running libp2p node for the Borgkit mesh.
///
/// Owns the Tokio runtime + Swarm event loop.
/// All operations are sent via an mpsc channel and executed on the loop thread.
pub struct BorgkitNode {
    cmd_tx:       mpsc::Sender<SwarmCmd>,
    peer_id:      PeerId,
    listen_addrs: Vec<Multiaddr>,
    /// Callback invoked for every incoming AgentRequest.
    /// Returns the raw JSON bytes of an AgentResponse.
    handler:      Option<Arc<dyn Fn(AgentRequest) -> AgentResponse + Send + Sync>>,
    _rt:          tokio::runtime::Runtime,
}

impl BorgkitNode {
    /// Build and start a new BorgkitNode, registering an optional request handler.
    pub fn new<F>(config: BorgkitNodeConfig, handler: Option<F>) -> anyhow::Result<Self>
    where
        F: Fn(AgentRequest) -> AgentResponse + Send + Sync + 'static,
    {
        let rt = tokio::runtime::Runtime::new()?;

        // Build swarm inside the runtime
        let (swarm, peer_id) = rt.block_on(async { build_swarm().await })?;
        let listen_addrs = swarm.listeners().cloned().collect();

        let (cmd_tx, cmd_rx) = mpsc::channel::<SwarmCmd>(256);

        let h: Option<Arc<dyn Fn(AgentRequest) -> AgentResponse + Send + Sync>> =
            handler.map(|f| Arc::new(f) as _);
        let handler_clone = h.clone();

        // Spawn the swarm event loop
        rt.spawn(swarm_loop(swarm, cmd_rx, handler_clone, config.listen_addrs));

        Ok(Self {
            cmd_tx,
            peer_id,
            listen_addrs,
            handler: h,
            _rt: rt,
        })
    }

    pub fn peer_id(&self) -> &PeerId     { &self.peer_id }
    pub fn listen_addrs(&self) -> &[Multiaddr] { &self.listen_addrs }

    /// Dial a remote multiaddr.
    pub fn dial(&self, addr: Multiaddr) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.blocking_send(SwarmCmd::Dial { addr, result: tx }).map_err(|e| e.to_string())?;
        rx.blocking_recv().map_err(|e| e.to_string())?
    }

    /// Send an AgentRequest to a known peer and wait for the AgentResponse.
    pub fn send(&self, peer: PeerId, request: AgentRequest) -> Result<AgentResponse, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.blocking_send(SwarmCmd::Send { peer, request, result: tx }).map_err(|e| e.to_string())?;
        rx.blocking_recv().map_err(|e| e.to_string())?
    }

    /// Publish a gossip message on the Borgkit topic.
    pub fn publish(&self, data: Vec<u8>) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.blocking_send(SwarmCmd::Publish { data, result: tx }).map_err(|e| e.to_string())?;
        rx.blocking_recv().map_err(|e| e.to_string())?.map(|_| ())
    }
}

// ── swarm construction ────────────────────────────────────────────────────────

async fn build_swarm() -> anyhow::Result<(Swarm<BorgkitBehaviour>, PeerId)> {
    let swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key| {
            // Invoke: request/response
            let invoke = request_response::Behaviour::new(
                vec![(StreamProtocol::new(INVOKE_PROTO), ProtocolSupport::Full)],
                request_response::Config::default(),
            );

            // GossipSub
            let gs_cfg = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(10))
                .build()
                .expect("GossipSub config");
            let mut gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gs_cfg,
            ).expect("GossipSub behaviour");
            let topic = gossipsub::IdentTopic::new(GOSSIP_TOPIC);
            gossipsub.subscribe(&topic).expect("gossip subscribe");

            // Identify
            let identify = identify::Behaviour::new(identify::Config::new(
                "/borgkit/1.0.0".into(),
                key.public(),
            ));

            Ok(BorgkitBehaviour { invoke, gossipsub, identify })
        })?
        .build();

    let peer_id = *swarm.local_peer_id();
    Ok((swarm, peer_id))
}

// ── swarm event loop ──────────────────────────────────────────────────────────

async fn swarm_loop(
    mut swarm:      Swarm<BorgkitBehaviour>,
    mut cmd_rx:     mpsc::Receiver<SwarmCmd>,
    handler:        Option<Arc<dyn Fn(AgentRequest) -> AgentResponse + Send + Sync>>,
    listen_addrs:   Vec<Multiaddr>,
) {
    for addr in listen_addrs {
        let _ = swarm.listen_on(addr);
    }

    // Map request_response::OutboundRequestId → oneshot::Sender
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<AgentResponse, String>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    None => break,
                    Some(SwarmCmd::Dial { addr, result }) => {
                        let r = swarm.dial(addr).map_err(|e| e.to_string());
                        let _ = result.send(r);
                    }
                    Some(SwarmCmd::Send { peer, request, result }) => {
                        let id = swarm.behaviour_mut().invoke.send_request(&peer, request);
                        pending.lock().unwrap().insert(id.0, result);
                    }
                    Some(SwarmCmd::Publish { data, result }) => {
                        let topic = gossipsub::IdentTopic::new(GOSSIP_TOPIC);
                        let r = swarm.behaviour_mut().gossipsub
                            .publish(topic, data)
                            .map_err(|e| format!("{e:?}"));
                        let _ = result.send(r);
                    }
                    Some(SwarmCmd::PeerInfo { result }) => {
                        let pid   = *swarm.local_peer_id();
                        let addrs = swarm.listeners().cloned().collect();
                        let _ = result.send((pid, addrs));
                    }
                }
            }

            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(BorgkitBehaviourEvent::Invoke(
                        request_response::Event::Message {
                            peer,
                            message: request_response::Message::Request { request, channel, .. },
                        }
                    )) => {
                        let resp = if let Some(ref h) = handler {
                            h(request)
                        } else {
                            AgentResponse::error("", "no handler registered")
                        };
                        let _ = swarm.behaviour_mut().invoke.send_response(channel, resp);
                    }

                    SwarmEvent::Behaviour(BorgkitBehaviourEvent::Invoke(
                        request_response::Event::Message {
                            message: request_response::Message::Response { request_id, response },
                            ..
                        }
                    )) => {
                        if let Some(tx) = pending.lock().unwrap().remove(&request_id.0) {
                            let _ = tx.send(Ok(response));
                        }
                    }

                    SwarmEvent::Behaviour(BorgkitBehaviourEvent::Invoke(
                        request_response::Event::OutboundFailure { request_id, error, .. }
                    )) => {
                        if let Some(tx) = pending.lock().unwrap().remove(&request_id.0) {
                            let _ = tx.send(Err(format!("{error:?}")));
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}
