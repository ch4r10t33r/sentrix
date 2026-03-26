//! sentrix-libp2p — libp2p transport for the Sentrix agent mesh.
//!
//! Exposes:
//!   - Rust-native API via [`SentrixNode`]
//!   - C FFI via `ffi` module (for Python ctypes / Zig @cImport)
//!
//! Protocols:
//!   /sentrix/invoke/1.0.0   — request/response (LP-framed JSON)
//!   /sentrix/gossip/1.0.0   — GossipSub topic
//!   /sentrix/stream/1.0.0   — streaming chunks (future)

pub mod node;
pub mod invoke;
pub mod gossip;
pub mod ffi;

pub use node::{SentrixNode, SentrixNodeConfig};
pub use invoke::InvokeCodec;
pub use gossip::GOSSIP_TOPIC;
