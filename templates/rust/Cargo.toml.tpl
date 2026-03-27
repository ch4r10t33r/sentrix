[package]
name = "{{PROJECT_NAME}}"
version = "0.1.0"
edition = "2021"
description = "Borgkit agent project — ERC-8004 compliant"

[[bin]]
name = "{{PROJECT_NAME}}"
path = "src/main.rs"

[lib]
path = "src/lib.rs"

[[example]]
name = "did_key_identity"
path = "examples/did_key_identity.rs"

[[example]]
name = "gossip_fanout_discovery"
path = "examples/gossip_fanout_discovery.rs"

[dependencies]
async-trait  = "0.1"
serde        = { version = "1",   features = ["derive"] }
serde_json   = "1"
tokio        = { version = "1",   features = ["full"] }
chrono       = { version = "0.4", features = ["serde"] }
axum         = "0.7"
tower        = "0.4"
uuid         = { version = "1",   features = ["v4"] }
reqwest      = { version = "0.12", features = ["json"] }
urlencoding  = "2"
sha2         = "0.10"
base64       = "0.22"
bs58         = "0.5"

# ── ANR (Agent Network Record) ────────────────────────────────────────────────
k256         = { version = "0.13", features = ["ecdsa"] }
sha3         = "0.10"

# ── libp2p P2P discovery ──────────────────────────────────────────────────────
# Feature flags:
#   quic      → QUIC transport (quinn-based, no TCP listener needed)
#   kad       → Kademlia DHT for capability-keyed discovery
#   mdns      → mDNS for zero-config LAN discovery
#   identify  → peer protocol/address exchange
#   dcutr     → hole punching (Direct Connection Upgrade through Relay)
#   relay     → circuit relay v2 fallback for strict NATs
#   noise     → encryption for relay connections (relay uses TCP)
#   yamux     → stream multiplexer for relay connections
#   tcp       → outbound relay connections only (no TCP listener opened)
#   secp256k1 → use ANR key as libp2p identity (same keypair, one identity)
#   tokio     → tokio runtime integration
#   macros    → NetworkBehaviour derive macro
[dependencies.libp2p]
version  = "0.54"
features = [
  "quic",
  "kad",
  "mdns",
  "identify",
  "dcutr",
  "relay",
  "noise",
  "yamux",
  "tcp",
  "secp256k1",
  "tokio",
  "macros",
]
