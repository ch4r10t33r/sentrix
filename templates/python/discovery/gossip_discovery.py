"""
GossipDiscovery — capability propagation via peer-to-peer gossip fan-out.

Extends LocalDiscovery with gossip: when an agent registers or unregisters,
GossipDiscovery fans out an announce/revoke GossipMessage to all known peers
over HTTP.  Incoming gossip messages update the local registry and are
re-forwarded with ttl-1 until the message has traversed the configured
number of hops.

Usage
-----
    from discovery.gossip_discovery import GossipDiscovery

    registry = GossipDiscovery(agent_id="borgkit://agent/me")
    await registry.add_peer("borgkit://agent/peer-a", "http://peer-a:6174")
    await registry.register(my_entry)   # auto-gossips to peers
"""

from __future__ import annotations

import asyncio
import json
import warnings
from typing import Callable, Awaitable, Dict, List, Optional
from datetime import datetime, timezone

from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus, IAgentDiscovery
from interfaces.iagent_mesh import GossipMessage, IGossipProtocol, GossipHandler


class GossipDiscovery(IAgentDiscovery, IGossipProtocol):
    """
    In-memory discovery registry with gossip-based propagation.

    Inherits:
        IAgentDiscovery  — register, unregister, query, list_all, heartbeat
        IGossipProtocol  — broadcast, receive, subscribe, peers, add_peer, remove_peer
    """

    def __init__(self, agent_id: str, default_ttl: int = 3):
        self._agent_id   = agent_id
        self._default_ttl = default_ttl
        self._registry: Dict[str, DiscoveryEntry] = {}
        self._peers:    Dict[str, str] = {}          # agent_id → http endpoint
        self._handlers: List[GossipHandler] = []
        self._seen:     set = set()                  # nonces / (sender+timestamp) seen

    # ── IAgentDiscovery ───────────────────────────────────────────────────────

    async def register(self, entry: DiscoveryEntry) -> None:
        self._registry[entry.agent_id] = entry
        import dataclasses
        msg = GossipMessage(
            type="announce",
            sender_id=self._agent_id,
            ttl=self._default_ttl,
            entry=dataclasses.asdict(entry),
        )
        await self.broadcast(msg)

    async def unregister(self, agent_id: str) -> None:
        entry = self._registry.pop(agent_id, None)
        if entry is None:
            return
        import dataclasses
        msg = GossipMessage(
            type="revoke",
            sender_id=self._agent_id,
            ttl=self._default_ttl,
            entry=dataclasses.asdict(entry),
        )
        await self.broadcast(msg)

    async def query(self, capability: str) -> List[DiscoveryEntry]:
        return [
            e for e in self._registry.values()
            if capability in e.capabilities and e.health.status != "unhealthy"
        ]

    async def list_all(self) -> List[DiscoveryEntry]:
        return list(self._registry.values())

    async def heartbeat(self, agent_id: str) -> None:
        if agent_id in self._registry:
            self._registry[agent_id].health = HealthStatus(
                status="healthy",
                last_heartbeat=datetime.now(timezone.utc).isoformat(),
            )
        msg = GossipMessage(
            type="heartbeat",
            sender_id=agent_id,
            ttl=1,   # heartbeat gossip only travels 1 hop
        )
        await self.broadcast(msg)

    # ── IGossipProtocol ───────────────────────────────────────────────────────

    async def broadcast(self, message: GossipMessage) -> None:
        """Fan out message to all known peers via HTTP POST /gossip."""
        if not self._peers:
            return
        payload = json.dumps(message.to_dict()).encode()
        tasks = [
            self._send_gossip(endpoint, payload)
            for peer_id, endpoint in self._peers.items()
            if peer_id not in message.seen_by
        ]
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)

    async def receive(self, message: GossipMessage) -> None:
        """Process an incoming gossip message; update registry; re-forward if ttl > 0."""
        # Dedup
        dedup_key = f"{message.sender_id}:{message.timestamp}:{message.nonce}"
        if dedup_key in self._seen:
            return
        self._seen.add(dedup_key)
        # Keep seen set bounded
        if len(self._seen) > 10_000:
            self._seen = set(list(self._seen)[-5_000:])

        # Apply to local registry
        if message.type == "announce" and message.entry:
            entry = _entry_from_dict(message.entry)
            if entry:
                self._registry[entry.agent_id] = entry

        elif message.type == "revoke" and message.entry:
            agent_id = message.entry.get("agent_id", "")
            self._registry.pop(agent_id, None)

        elif message.type == "heartbeat":
            if message.sender_id in self._registry:
                self._registry[message.sender_id].health = HealthStatus(
                    status="healthy",
                    last_heartbeat=datetime.now(timezone.utc).isoformat(),
                )

        # Invoke subscribed handlers
        for handler in self._handlers:
            try:
                await handler(message)
            except Exception as e:
                warnings.warn(f"[GossipDiscovery] handler error: {e}")

        # Forward if hops remain
        if message.should_forward:
            await self.broadcast(message.forwarded_by(self._agent_id))

    def subscribe(self, handler: GossipHandler) -> None:
        self._handlers.append(handler)

    def peers(self) -> List[str]:
        return list(self._peers.keys())

    async def add_peer(self, agent_id: str, endpoint: str) -> None:
        self._peers[agent_id] = endpoint

    async def remove_peer(self, agent_id: str) -> None:
        self._peers.pop(agent_id, None)

    # ── internal ──────────────────────────────────────────────────────────────

    @staticmethod
    async def _send_gossip(endpoint: str, payload: bytes) -> None:
        """POST gossip payload to peer's /gossip endpoint."""
        url = endpoint.rstrip("/") + "/gossip"
        try:
            import httpx
            async with httpx.AsyncClient(timeout=3.0) as client:
                await client.post(url, content=payload, headers={"Content-Type": "application/json"})
            return
        except ImportError:
            pass
        try:
            import aiohttp
            async with aiohttp.ClientSession() as session:
                await session.post(url, data=payload, headers={"Content-Type": "application/json"},
                                   timeout=aiohttp.ClientTimeout(total=3.0))
            return
        except ImportError:
            pass
        import urllib.request
        req = urllib.request.Request(url, data=payload,
                                     headers={"Content-Type": "application/json"}, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=3):
                pass
        except Exception:
            pass


def _entry_from_dict(d: dict) -> Optional[DiscoveryEntry]:
    try:
        net = d.get("network", {})
        return DiscoveryEntry(
            agent_id=d["agent_id"],
            name=d.get("name", ""),
            owner=d.get("owner", "anonymous"),
            capabilities=d.get("capabilities", []),
            network=NetworkInfo(
                protocol=net.get("protocol", "http"),
                host=net.get("host", "localhost"),
                port=net.get("port", 6174),
                tls=net.get("tls", False),
            ),
            health=HealthStatus(
                status=d.get("health", {}).get("status", "healthy"),
                last_heartbeat=d.get("health", {}).get("last_heartbeat", ""),
            ),
            registered_at=d.get("registered_at", ""),
            metadata_uri=d.get("metadata_uri"),
        )
    except Exception:
        return None
