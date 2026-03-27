//! inai-libp2p — libp2p transport for the Inai agent mesh.
//!
//! Exposes:
//!   - Rust-native API via [`InaiNode`]
//!   - C FFI via `ffi` module (for Python ctypes / Zig @cImport)
//!
//! Protocols:
//!   /inai/invoke/1.0.0   — request/response (LP-framed JSON)
//!   /inai/gossip/1.0.0   — GossipSub topic
//!   /inai/stream/1.0.0   — streaming chunks (future)

pub mod node;
pub mod invoke;
pub mod gossip;
pub mod ffi;

pub use node::{InaiNode, InaiNodeConfig};
pub use invoke::InvokeCodec;
pub use gossip::GOSSIP_TOPIC;
