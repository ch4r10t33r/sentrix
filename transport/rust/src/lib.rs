//! borgkit-libp2p — libp2p transport for the Borgkit agent mesh.
//!
//! Exposes:
//!   - Rust-native API via [`BorgkitNode`]
//!   - C FFI via `ffi` module (for Python ctypes / Zig @cImport)
//!
//! Protocols:
//!   /borgkit/invoke/1.0.0   — request/response (LP-framed JSON)
//!   /borgkit/gossip/1.0.0   — GossipSub topic
//!   /borgkit/stream/1.0.0   — streaming chunks (future)

pub mod node;
pub mod invoke;
pub mod gossip;
pub mod ffi;

pub use node::{BorgkitNode, BorgkitNodeConfig};
pub use invoke::InvokeCodec;
pub use gossip::GOSSIP_TOPIC;
