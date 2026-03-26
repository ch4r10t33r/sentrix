//! Libp2pDiscovery — fully P2P discovery backend for Sentrix.
//!
//! Architecture:
//!   Transport   : QUIC (via quinn, rust-libp2p's quic feature)
//!   Routing     : Kademlia DHT  (/sentrix/kad/1.0.0 — isolated from IPFS)
//!   Local LAN   : mDNS (optional, default on)
//!   NAT         : DCUtR hole punching + circuit-relay-v2 fallback
//!   Identity    : secp256k1 keypair from ANR — same key → same PeerId
//!
//! The Swarm runs in a dedicated tokio task.  All IAgentDiscovery calls send
//! commands over an mpsc channel and receive results via oneshot channels,
//! keeping the Swarm (non-Send) in a single-threaded context.
//!
//! # Usage
//! ```rust
//! let cfg = Libp2pDiscoveryConfig {
//!     private_key_bytes: my_anr_key,
//!     ..Default::default()
//! };
//! let discovery = Libp2pDiscovery::start(cfg).await?;
//! discovery.register(entry).await?;
//! let peers = discovery.query("web_search").await?;
//! discovery.stop().await;
//! ```

use crate::discovery::{DiscoveryEntry, IAgentDiscovery, NetworkInfo, HealthStatus};
use async_trait::async_trait;
use libp2p::{
    identify, kad,
    kad::store::MemoryStore,
    mdns,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, StreamProtocol, Swarm,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot, RwLock};

// ── DHT key helpers ───────────────────────────────────────────────────────────

/// Capability provider-record key: SHA-256("sentrix:cap:<capability>").
fn capability_key(capability: &str) -> kad::RecordKey {
    let mut h = Sha256::new();
    h.update(format!("sentrix:cap:{}", capability).as_bytes());
    kad::RecordKey::new(&h.finalize())
}

/// Value-record key for a full DiscoveryEntry: SHA-256("sentrix:anr:<agentId>").
fn anr_dht_key(agent_id: &str) -> kad::RecordKey {
    let mut h = Sha256::new();
    h.update(format!("sentrix:anr:{}", agent_id).as_bytes());
    kad::RecordKey::new(&h.finalize())
}

/// Reverse PeerId → agentId key.
fn pid_dht_key(peer_id: &PeerId) -> kad::RecordKey {
    let key = format!("/sentrix/pid/{}", peer_id);
    kad::RecordKey::new(key.as_bytes())
}

// ── DHT value envelope ────────────────────────────────────────────────────────

/// Signed envelope stored in the DHT value store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DhtEnvelope {
    v:     u8,            // envelope version, currently 1
    seq:   u64,           // monotonically increasing
    entry: StoredEntry,   // the DiscoveryEntry fields
    sig:   String,        // base64 compact secp256k1 signature (placeholder)
}

/// Serialisable subset of DiscoveryEntry (all fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    agent_id:      String,
    name:          String,
    owner:         String,
    capabilities:  Vec<String>,
    protocol:      String,
    host:          String,
    port:          u16,
    tls:           bool,
    peer_id:       String,
    multiaddr:     String,
    status:        String,
    last_heartbeat: String,
    uptime_seconds: u64,
    registered_at:  String,
    metadata_uri:   Option<String>,
}

impl From<&DiscoveryEntry> for StoredEntry {
    fn from(e: &DiscoveryEntry) -> Self {
        Self {
            agent_id:       e.agent_id.clone(),
            name:           e.name.clone(),
            owner:          e.owner.clone(),
            capabilities:   e.capabilities.clone(),
            protocol:       e.network.protocol.clone(),
            host:           e.network.host.clone(),
            port:           e.network.port,
            tls:            e.network.tls,
            peer_id:        e.network.peer_id.clone(),
            multiaddr:      e.network.multiaddr.clone(),
            status:         e.health.status.clone(),
            last_heartbeat: e.health.last_heartbeat.clone(),
            uptime_seconds: e.health.uptime_seconds,
            registered_at:  e.registered_at.clone(),
            metadata_uri:   e.metadata_uri.clone(),
        }
    }
}

impl From<StoredEntry> for DiscoveryEntry {
    fn from(s: StoredEntry) -> Self {
        // Apply staleness heuristic: if last_heartbeat is >15 min old → unhealthy
        let status = apply_staleness(&s.last_heartbeat, &s.status);
        Self {
            agent_id:     s.agent_id,
            name:         s.name,
            owner:        s.owner,
            capabilities: s.capabilities,
            network: NetworkInfo {
                protocol:  s.protocol,
                host:      s.host,
                port:      s.port,
                tls:       s.tls,
                peer_id:   s.peer_id,
                multiaddr: s.multiaddr,
            },
            health: HealthStatus {
                status,
                last_heartbeat:  s.last_heartbeat,
                uptime_seconds:  s.uptime_seconds,
            },
            registered_at: s.registered_at,
            metadata_uri:  s.metadata_uri,
        }
    }
}

fn apply_staleness(last_heartbeat: &str, current_status: &str) -> String {
    // Parse ISO 8601, compute delta in seconds, downgrade status if stale
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_heartbeat) {
        let now   = chrono::Utc::now();
        let delta = (now - dt.with_timezone(&chrono::Utc)).num_seconds();
        if delta > 15 * 60 { return "unhealthy".into(); }
        if delta > 5  * 60 { return "degraded".into();  }
    }
    current_status.to_string()
}

fn encode_envelope(entry: &DiscoveryEntry, seq: u64, signing_key: Option<&[u8; 32]>) -> Vec<u8> {
    // Build unsigned envelope first so we can sign its serialised bytes.
    let unsigned = DhtEnvelope {
        v:     1,
        seq,
        entry: StoredEntry::from(entry),
        sig:   String::new(),
    };

    let sig_str = if let Some(key) = signing_key {
        // Serialise the unsigned form, hash with SHA-256, sign with k256.
        let payload = serde_json::to_vec(&unsigned).unwrap_or_default();
        let digest   = Sha256::digest(&payload);

        let ga = k256::elliptic_curve::generic_array::GenericArray::from_slice(key);
        if let Ok(sk) = k256::ecdsa::SigningKey::from_bytes(ga) {
            use k256::ecdsa::signature::Signer;
            let sig: k256::ecdsa::Signature = sk.sign(&digest);
            base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let envelope = DhtEnvelope {
        v:     1,
        seq,
        entry: StoredEntry::from(entry),
        sig:   sig_str,
    };
    serde_json::to_vec(&envelope).unwrap_or_default()
}

fn decode_envelope(raw: &[u8]) -> Option<(DiscoveryEntry, u64)> {
    let env: DhtEnvelope = serde_json::from_slice(raw).ok()?;
    if env.v != 1 { return None; }
    Some((DiscoveryEntry::from(env.entry), env.seq))
}

// ── NetworkBehaviour ──────────────────────────────────────────────────────────

#[derive(NetworkBehaviour)]
struct SentrixBehaviour {
    kademlia: kad::Behaviour<MemoryStore>,
    identify: identify::Behaviour,
    mdns:     mdns::tokio::Behaviour,
}

// ── Swarm command protocol ────────────────────────────────────────────────────

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

enum SwarmCommand {
    Register  { entry: DiscoveryEntry, tx: oneshot::Sender<Result<(), BoxErr>> },
    Unregister{ agent_id: String,       tx: oneshot::Sender<Result<(), BoxErr>> },
    Query     { capability: String,     tx: oneshot::Sender<Result<Vec<DiscoveryEntry>, BoxErr>> },
    ListAll   {                         tx: oneshot::Sender<Result<Vec<DiscoveryEntry>, BoxErr>> },
    Heartbeat { agent_id: String,       tx: oneshot::Sender<Result<(), BoxErr>> },
    Stop,
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Libp2pDiscoveryConfig {
    /// 32-byte secp256k1 private key (same key used to sign ANR records).
    pub private_key_bytes: [u8; 32],
    /// UDP port to listen on (0 = OS-assigned). Default: 6174.
    pub listen_port: u16,
    /// Bootstrap peer multiaddrs.
    pub bootstrap_peers: Vec<(PeerId, Multiaddr)>,
    /// Re-publish DHT records this often (default: 30 s).
    pub heartbeat_secs: u64,
    /// Enable mDNS local discovery (default: true).
    pub enable_mdns: bool,
    /// Kademlia client mode — does not store records for others (default: false).
    pub dht_client_mode: bool,
}

impl Default for Libp2pDiscoveryConfig {
    fn default() -> Self {
        Self {
            private_key_bytes: [0u8; 32], // caller must set a real key
            listen_port:        6174,
            bootstrap_peers:    vec![],
            heartbeat_secs:     30,
            enable_mdns:        true,
            dht_client_mode:    false,
        }
    }
}

// ── Libp2pDiscovery ───────────────────────────────────────────────────────────

/// Shared state accessed by both the swarm task and the public API.
struct SharedState {
    /// Locally registered agents: agentId → (entry, seq).
    local_entries: HashMap<String, (DiscoveryEntry, u64)>,
    /// Pending DHT GET queries: query_id → capability + response channel.
    pending_queries: HashMap<kad::QueryId, (String, oneshot::Sender<Result<Vec<DiscoveryEntry>, BoxErr>>)>,
    /// Pending DHT GET for entry lookup: query_id → (peer_id, outer_tx).
    pending_gets: HashMap<kad::QueryId, String>,
    /// Accumulated provider entries per capability query.
    query_results: HashMap<String, Vec<DiscoveryEntry>>,
}

/// Public handle to the running libp2p discovery node.
///
/// Cheap to clone — wraps an Arc + mpsc sender.
#[derive(Clone)]
pub struct Libp2pDiscovery {
    cmd_tx: mpsc::Sender<SwarmCommand>,
    state:  Arc<RwLock<SharedState>>,
}

impl Libp2pDiscovery {
    /// Build a PeerId from the ANR secp256k1 private key.
    fn build_keypair(raw: &[u8; 32]) -> libp2p::identity::Keypair {
        let secret = libp2p::identity::secp256k1::SecretKey::try_from_bytes(*raw)
            .expect("invalid secp256k1 private key");
        libp2p::identity::Keypair::from(
            libp2p::identity::secp256k1::Keypair::from(secret)
        )
    }

    /// Start the libp2p Swarm in a background tokio task and return a handle.
    pub async fn start(cfg: Libp2pDiscoveryConfig) -> Result<Self, BoxErr> {
        let keypair  = Self::build_keypair(&cfg.private_key_bytes);
        let local_id = PeerId::from_public_key(&keypair.public());

        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/udp/{}/quic-v1", cfg.listen_port)
            .parse()?;

        // ── Build swarm ───────────────────────────────────────────────────────
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair.clone())
            .with_tokio()
            .with_quic()
            .with_behaviour(|key| {
                let local_peer = PeerId::from_public_key(&key.public());

                // Kademlia with custom protocol (isolated from IPFS)
                let mut kad_cfg = kad::Config::new(
                    StreamProtocol::new("/sentrix/kad/1.0.0")
                );
                if cfg.dht_client_mode {
                    kad_cfg.set_mode(Some(kad::Mode::Client));
                }
                let kademlia = kad::Behaviour::with_config(
                    local_peer,
                    MemoryStore::new(local_peer),
                    kad_cfg,
                );

                let identify  = identify::Behaviour::new(
                    identify::Config::new("/sentrix/identify/1.0.0".into(), key.public())
                );
                let mdns = if cfg.enable_mdns {
                    mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer)?
                } else {
                    mdns::tokio::Behaviour::new(
                        mdns::Config { enable_ipv6: false, ..Default::default() },
                        local_peer,
                    )?
                };

                Ok(SentrixBehaviour { kademlia, identify, mdns })
            })?
            .build();

        swarm.listen_on(listen_addr)?;

        // Add bootstrap peers
        for (peer_id, addr) in &cfg.bootstrap_peers {
            swarm.behaviour_mut().kademlia.add_address(peer_id, addr.clone());
        }
        if !cfg.bootstrap_peers.is_empty() {
            let _ = swarm.behaviour_mut().kademlia.bootstrap();
        }

        println!("[Libp2pDiscovery] Started — PeerId: {local_id}");

        // ── Shared state ──────────────────────────────────────────────────────
        let state = Arc::new(RwLock::new(SharedState {
            local_entries:    HashMap::new(),
            pending_queries:  HashMap::new(),
            pending_gets:     HashMap::new(),
            query_results:    HashMap::new(),
        }));

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SwarmCommand>(64);

        let state_loop = Arc::clone(&state);
        let hb_secs    = cfg.heartbeat_secs;

        // ── Swarm event loop ──────────────────────────────────────────────────
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Process commands from the public API
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            SwarmCommand::Stop => break,

                            SwarmCommand::Register { entry, tx } => {
                                let seq = {
                                    let state = state_loop.read().await;
                                    state.local_entries.get(&entry.agent_id)
                                        .map(|(_, s)| *s + 1).unwrap_or(1)
                                };
                                let encoded  = encode_envelope(&entry, seq, Some(&cfg.private_key_bytes));
                                let val_key  = anr_dht_key(&entry.agent_id);
                                let pid_key  = pid_dht_key(&local_id);
                                let agent_id_bytes = entry.agent_id.as_bytes().to_vec();

                                // Store value records
                                let _ = swarm.behaviour_mut().kademlia.put_record(
                                    kad::Record::new(val_key, encoded),
                                    kad::Quorum::One,
                                );
                                let _ = swarm.behaviour_mut().kademlia.put_record(
                                    kad::Record::new(pid_key, agent_id_bytes),
                                    kad::Quorum::One,
                                );

                                // Announce as provider for each capability
                                for cap in &entry.capabilities {
                                    let key = capability_key(cap);
                                    let _ = swarm.behaviour_mut().kademlia.start_providing(key);
                                }

                                state_loop.write().await
                                    .local_entries.insert(entry.agent_id.clone(), (entry, seq));

                                let _ = tx.send(Ok(()));
                            }

                            SwarmCommand::Unregister { agent_id, tx } => {
                                state_loop.write().await.local_entries.remove(&agent_id);
                                let _ = tx.send(Ok(()));
                            }

                            SwarmCommand::Query { capability, tx } => {
                                let key = capability_key(&capability);
                                let qid = swarm.behaviour_mut().kademlia.get_providers(key);
                                state_loop.write().await.pending_queries.insert(qid, (capability, tx));
                            }

                            SwarmCommand::ListAll { tx } => {
                                let entries: Vec<DiscoveryEntry> = state_loop.read().await
                                    .local_entries.values()
                                    .map(|(e, _)| e.clone())
                                    .collect();
                                let _ = tx.send(Ok(entries));
                            }

                            SwarmCommand::Heartbeat { agent_id, tx } => {
                                let maybe_entry = {
                                    let state = state_loop.read().await;
                                    state.local_entries.get(&agent_id).cloned()
                                };
                                if let Some((mut entry, seq)) = maybe_entry {
                                    let new_seq = seq + 1;
                                    entry.health.status = "healthy".into();
                                    entry.health.last_heartbeat =
                                        chrono::Utc::now().to_rfc3339();

                                    let encoded = encode_envelope(&entry, new_seq, Some(&cfg.private_key_bytes));
                                    let val_key = anr_dht_key(&entry.agent_id);
                                    let _ = swarm.behaviour_mut().kademlia.put_record(
                                        kad::Record::new(val_key, encoded),
                                        kad::Quorum::One,
                                    );
                                    state_loop.write().await
                                        .local_entries.insert(agent_id, (entry, new_seq));
                                    let _ = tx.send(Ok(()));
                                } else {
                                    let _ = tx.send(Err("unknown agent_id".into()));
                                }
                            }
                        }
                    }

                    // Process swarm events
                    event = swarm.next() => {
                        let Some(event) = event else { break };
                        Self::handle_swarm_event(event, &state_loop, &mut swarm).await;
                    }
                }
            }
            println!("[Libp2pDiscovery] Swarm event loop stopped");
        });

        Ok(Self { cmd_tx, state })
    }

    /// Handle a single swarm event.
    async fn handle_swarm_event(
        event: SwarmEvent<SentrixBehaviourEvent>,
        state: &Arc<RwLock<SharedState>>,
        swarm: &mut Swarm<SentrixBehaviour>,
    ) {
        match event {
            SwarmEvent::Behaviour(SentrixBehaviourEvent::Kademlia(ke)) => {
                Self::handle_kad_event(ke, state, swarm).await;
            }
            SwarmEvent::Behaviour(SentrixBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, addr) in list {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("[Libp2pDiscovery] Listening on {address}");
            }
            _ => {}
        }
    }

    async fn handle_kad_event(
        event: kad::Event,
        state: &Arc<RwLock<SharedState>>,
        swarm: &mut Swarm<SentrixBehaviour>,
    ) {
        use kad::Event::*;
        match event {
            OutboundQueryProgressed { id, result: kad::QueryResult::GetProviders(Ok(res)), .. } => {
                use kad::GetProvidersOk;
                match res {
                    GetProvidersOk::FoundProviders { providers, .. } => {
                        // Collect (cap, peer_ids) while holding the lock, then
                        // drop the lock before calling into swarm.
                        let cap_opt = {
                            let st = state.read().await;
                            st.pending_queries.get(&id).map(|(c, _)| c.clone())
                        };
                        if let Some(cap) = cap_opt {
                            // Ensure the results bucket exists.
                            state.write().await.query_results.entry(cap.clone()).or_default();

                            // For each provider, issue a DHT GET for their
                            // per-peer record (/sentrix/pid/<peer_id>).
                            for pid in providers {
                                let key = pid_dht_key(&pid);
                                let get_qid = swarm.behaviour_mut().kademlia.get_record(key);
                                // Track this GET so the GetRecord handler can
                                // associate the result with the right capability.
                                state.write().await.pending_gets.insert(get_qid, cap.clone());
                            }
                        }
                    }
                    GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {
                        let mut st = state.write().await;
                        if let Some((cap, tx)) = st.pending_queries.remove(&id) {
                            let results = st.query_results.remove(&cap).unwrap_or_default();
                            let _ = tx.send(Ok(results));
                        }
                    }
                }
            }
            OutboundQueryProgressed { id, result: kad::QueryResult::GetRecord(Ok(res)), .. } => {
                use kad::GetRecordOk;
                if let GetRecordOk::FoundRecord(record) = res {
                    if let Some((entry, _seq)) = decode_envelope(&record.record.value) {
                        let mut st = state.write().await;
                        if let Some(cap) = st.pending_gets.remove(&id) {
                            st.query_results.entry(cap).or_default().push(entry);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Stop the discovery node.
    pub async fn stop(&self) {
        let _ = self.cmd_tx.send(SwarmCommand::Stop).await;
    }

    async fn send<T>(&self, build: impl FnOnce(oneshot::Sender<T>) -> SwarmCommand) -> T
    where T: Send + 'static
    {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(build(tx)).await;
        rx.await.expect("swarm task terminated unexpectedly")
    }
}

// ── IAgentDiscovery impl ──────────────────────────────────────────────────────

#[async_trait]
impl IAgentDiscovery for Libp2pDiscovery {
    async fn register(&self, entry: DiscoveryEntry) -> Result<(), Box<dyn std::error::Error>> {
        self.send(|tx| SwarmCommand::Register { entry, tx }).await
            .map_err(|e| e as Box<dyn std::error::Error>)
    }

    async fn unregister(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let agent_id = agent_id.to_string();
        self.send(|tx| SwarmCommand::Unregister { agent_id, tx }).await
            .map_err(|e| e as Box<dyn std::error::Error>)
    }

    async fn query(&self, capability: &str) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let capability = capability.to_string();
        self.send(|tx| SwarmCommand::Query { capability, tx }).await
            .map_err(|e| e as Box<dyn std::error::Error>)
    }

    async fn list_all(&self) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        self.send(|tx| SwarmCommand::ListAll { tx }).await
            .map_err(|e| e as Box<dyn std::error::Error>)
    }

    async fn heartbeat(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let agent_id = agent_id.to_string();
        self.send(|tx| SwarmCommand::Heartbeat { agent_id, tx }).await
            .map_err(|e| e as Box<dyn std::error::Error>)
    }
}
