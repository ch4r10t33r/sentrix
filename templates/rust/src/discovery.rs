use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub protocol: String, // "http" | "websocket" | "grpc" | "tcp" | "libp2p"
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub peer_id:   String,
    pub multiaddr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: String, // "healthy" | "degraded" | "unhealthy"
    pub last_heartbeat: String, // ISO 8601
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    pub agent_id: String,
    pub name: String,
    pub owner: String,
    pub capabilities: Vec<String>,
    pub network: NetworkInfo,
    pub health: HealthStatus,
    pub registered_at: String, // ISO 8601
    pub metadata_uri: Option<String>,
}

/// Discovery layer trait.
/// Swap the implementation:
///   LocalDiscovery   → in-memory (dev)
///   HttpDiscovery    → REST registry
///   GossipDiscovery  → P2P gossip
///   OnChainDiscovery → ERC-8004 Ethereum
#[async_trait]
pub trait IAgentDiscovery: Send + Sync {
    async fn register(&self, entry: DiscoveryEntry) -> Result<(), Box<dyn std::error::Error>>;
    async fn unregister(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>>;
    async fn query(&self, capability: &str) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>>;
    async fn list_all(&self) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>>;
    async fn heartbeat(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>>;
}

// ── LocalDiscovery ────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
pub struct LocalDiscovery {
    registry: Arc<Mutex<HashMap<String, DiscoveryEntry>>>,
}

#[async_trait]
impl IAgentDiscovery for LocalDiscovery {
    async fn register(&self, entry: DiscoveryEntry) -> Result<(), Box<dyn std::error::Error>> {
        println!("[LocalDiscovery] Registered: {} ({:?})", entry.agent_id, entry.capabilities);
        self.registry.lock().unwrap().insert(entry.agent_id.clone(), entry);
        Ok(())
    }

    async fn unregister(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.registry.lock().unwrap().remove(agent_id);
        println!("[LocalDiscovery] Unregistered: {}", agent_id);
        Ok(())
    }

    async fn query(&self, capability: &str) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        let registry = self.registry.lock().unwrap();
        let results = registry
            .values()
            .filter(|e| e.capabilities.contains(&capability.to_string()) && e.health.status != "unhealthy")
            .cloned()
            .collect();
        Ok(results)
    }

    async fn list_all(&self) -> Result<Vec<DiscoveryEntry>, Box<dyn std::error::Error>> {
        Ok(self.registry.lock().unwrap().values().cloned().collect())
    }

    async fn heartbeat(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = self.registry.lock().unwrap();
        if let Some(entry) = registry.get_mut(agent_id) {
            entry.health.status = "healthy".into();
        }
        Ok(())
    }
}
