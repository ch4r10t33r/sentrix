//! Gossip helpers — topic name and message serialisation.

pub const GOSSIP_TOPIC: &str = "/borgkit/gossip/1.0.0";

use serde::{Deserialize, Serialize};

/// Wire-format for gossip messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipMessage {
    #[serde(rename = "type")]
    pub kind:      String,
    #[serde(rename = "senderId")]
    pub sender_id: String,
    pub timestamp: u64,
    pub ttl:       u32,
    #[serde(rename = "seenBy", default)]
    pub seen_by:   Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry:     Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default)]
    pub nonce:     String,
}

impl GossipMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// Return a copy with ttl decremented and agent_id appended to seen_by.
    pub fn forwarded_by(mut self, agent_id: &str) -> Self {
        self.ttl  = self.ttl.saturating_sub(1);
        self.seen_by.push(agent_id.to_string());
        self
    }
}
