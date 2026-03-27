"""
Libp2PAgentClient — IAgentClient over libp2p, backed by the Rust FFI.

Falls back to the standard HTTP AgentClient for entries that have no peerId.
"""
from __future__ import annotations

import json
import time
import uuid
from typing import Any, Dict, List, Optional, TYPE_CHECKING

from interfaces.iagent_client import IAgentClient, AgentClient
from interfaces.agent_request import AgentRequest
from interfaces.agent_response import AgentResponse
from interfaces.iagent_discovery import DiscoveryEntry, IAgentDiscovery
from .ffi import BorgkitLibp2P

if TYPE_CHECKING:
    from interfaces.iagent_mesh import (
        HeartbeatResponse, CapabilityExchangeResponse, AgentSession,
    )


class Libp2PAgentClient(IAgentClient):
    """
    IAgentClient that dispatches to libp2p peers (via Rust FFI) when the
    DiscoveryEntry carries a peerId, and falls back to HTTP otherwise.

    Usage
    -----
    ffi    = BorgkitLibp2P()
    ffi.start("/ip4/0.0.0.0/tcp/0")
    client = Libp2PAgentClient(ffi, discovery)
    resp   = await client.call_capability("weather_forecast", {"city": "NYC"})
    """

    def __init__(
        self,
        ffi:         BorgkitLibp2P,
        discovery:   IAgentDiscovery,
        *,
        caller_id:   str = "anonymous",
        timeout_ms:  int = 30_000,
    ) -> None:
        self._ffi       = ffi
        self._discovery = discovery
        self._caller_id = caller_id
        self._timeout   = timeout_ms
        self._http      = AgentClient(discovery, caller_id=caller_id, timeout_ms=timeout_ms)

    # ── lookup ─────────────────────────────────────────────────────────────────

    async def find(self, capability: str) -> Optional[DiscoveryEntry]:
        entries = await self._discovery.query(capability)
        healthy = [e for e in entries if e.health.status == "healthy"]
        return healthy[0] if healthy else (entries[0] if entries else None)

    async def find_all(self, capability: str) -> List[DiscoveryEntry]:
        entries = await self._discovery.query(capability)
        healthy = [e for e in entries if e.health.status == "healthy"]
        return healthy if healthy else entries

    async def find_by_id(self, agent_id: str) -> Optional[DiscoveryEntry]:
        all_entries = await self._discovery.list_all()
        for e in all_entries:
            if e.agent_id == agent_id:
                return e
        return None

    # ── interaction ────────────────────────────────────────────────────────────

    async def call(self, agent_id, capability, payload, *, caller_id="", timeout_ms=0):
        entry = await self.find_by_id(agent_id)
        if entry is None:
            return AgentResponse.error(str(uuid.uuid4()), f"Agent not found: {agent_id}")
        return await self.call_entry(entry, capability, payload,
                                     caller_id=caller_id or self._caller_id,
                                     timeout_ms=timeout_ms or self._timeout)

    async def call_capability(self, capability, payload, *, caller_id="", timeout_ms=0):
        entry = await self.find(capability)
        if entry is None:
            return AgentResponse.error(str(uuid.uuid4()), f"No agent for: {capability}")
        return await self.call_entry(entry, capability, payload,
                                     caller_id=caller_id or self._caller_id,
                                     timeout_ms=timeout_ms or self._timeout)

    async def call_entry(self, entry, capability, payload, *, caller_id="", timeout_ms=0):
        if entry.network.peer_id:
            return await self._dispatch_p2p(entry, capability, payload,
                                             caller_id or self._caller_id)
        return await self._http.call_entry(entry, capability, payload,
                                            caller_id=caller_id, timeout_ms=timeout_ms)

    # ── mesh (delegate to HTTP client) ─────────────────────────────────────────

    async def ping(self, agent_id, *, timeout_ms=5_000):
        return await self._http.ping(agent_id, timeout_ms=timeout_ms)

    async def connect(self, entry, *, timeout_ms=10_000):
        if entry.network.peer_id and entry.network.multiaddr:
            try:
                self._ffi.dial(entry.network.multiaddr)
            except ConnectionError:
                pass  # non-fatal — handshake below will surface any real error
        return await self._http.connect(entry, timeout_ms=timeout_ms)

    async def gossip_announce(self, entry, *, ttl=3):
        return await self._http.gossip_announce(entry, ttl=ttl)

    async def gossip_query(self, capability, *, ttl=3, timeout_ms=5_000):
        return await self._http.gossip_query(capability, ttl=ttl, timeout_ms=timeout_ms)

    # ── P2P dispatch ───────────────────────────────────────────────────────────

    async def _dispatch_p2p(
        self,
        entry:      DiscoveryEntry,
        capability: str,
        payload:    Dict[str, Any],
        caller_id:  str,
    ) -> AgentResponse:
        import asyncio
        req_dict = {
            "requestId":  str(uuid.uuid4()),
            "from":       caller_id,
            "capability": capability,
            "payload":    payload,
            "timestamp":  int(time.time() * 1000),
        }
        req_json = json.dumps(req_dict)
        loop = asyncio.get_event_loop()
        try:
            raw = await loop.run_in_executor(
                None,
                lambda: self._ffi.send(entry.network.peer_id, req_json),
            )
            d = json.loads(raw)
            return AgentResponse(
                request_id=d.get("requestId", req_dict["requestId"]),
                status=d.get("status", "error"),
                result=d.get("result"),
                error_message=d.get("errorMessage"),
                timestamp=d.get("timestamp", int(time.time() * 1000)),
            )
        except Exception as exc:
            return AgentResponse.error(req_dict["requestId"], str(exc))
