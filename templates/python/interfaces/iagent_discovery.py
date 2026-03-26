"""
Discovery layer interface.
Swap the implementation to change the backend:
  - LocalDiscovery    → in-memory (dev / testing)
  - HttpDiscovery     → REST-based registry
  - GossipDiscovery   → P2P gossip protocol
  - OnChainDiscovery  → ERC-8004 on-chain registry
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import List, Optional


@dataclass
class NetworkInfo:
    protocol: str   # "http" | "websocket" | "grpc" | "tcp"
    host: str
    port: int
    tls: bool = False
    peer_id: str = ""
    multiaddr: str = ""


@dataclass
class HealthStatus:
    status: str           # "healthy" | "degraded" | "unhealthy"
    last_heartbeat: str   # ISO 8601
    uptime_seconds: int = 0


@dataclass
class DiscoveryEntry:
    agent_id: str
    name: str
    owner: str
    capabilities: List[str]
    network: NetworkInfo
    health: HealthStatus
    registered_at: str        # ISO 8601
    metadata_uri: Optional[str] = None


class IAgentDiscovery(ABC):

    @abstractmethod
    async def register(self, entry: DiscoveryEntry) -> None:
        """Register an agent and its capabilities."""
        ...

    @abstractmethod
    async def unregister(self, agent_id: str) -> None:
        """Remove an agent from the discovery layer."""
        ...

    @abstractmethod
    async def query(self, capability: str) -> List[DiscoveryEntry]:
        """Find all agents that expose a given capability."""
        ...

    @abstractmethod
    async def list_all(self) -> List[DiscoveryEntry]:
        """List every registered agent."""
        ...

    @abstractmethod
    async def heartbeat(self, agent_id: str) -> None:
        """Emit a keep-alive so the registry knows the agent is alive."""
        ...

    async def find(self, capability: str) -> Optional[DiscoveryEntry]:
        """
        Return the best healthy agent for `capability`.
        Default implementation calls query() and picks the first healthy result.
        """
        entries = await self.query(capability)
        healthy = [e for e in entries if e.health.status == "healthy"]
        return healthy[0] if healthy else (entries[0] if entries else None)

    async def find_by_id(self, agent_id: str) -> Optional[DiscoveryEntry]:
        """
        Look up an agent by exact agent_id.
        Default implementation calls list_all() and filters.
        """
        for entry in await self.list_all():
            if entry.agent_id == agent_id:
                return entry
        return None
