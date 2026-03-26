"""
IAgentClient — standard interface for discovering and calling other Sentrix agents.
"""
from __future__ import annotations
from abc import ABC, abstractmethod
from typing import Any, AsyncGenerator, Dict, List, Optional, TYPE_CHECKING, Union
import uuid
import time

if TYPE_CHECKING:
    from .iagent_mesh import (
        HeartbeatResponse, CapabilityExchangeResponse, GossipMessage,
        HandshakeResult, AgentSession, StreamChunk, StreamEnd,
    )
    from .iagent_discovery import DiscoveryEntry

from .agent_request import AgentRequest
from .agent_response import AgentResponse
from .iagent_discovery import DiscoveryEntry, IAgentDiscovery


class IAgentClient(ABC):
    """
    Standard interface for discovering and calling Sentrix agents.

    Combines lookup (find by capability / agent ID) with invocation
    (send AgentRequest, receive AgentResponse) in a single coherent API.

    Concrete implementations handle transport (HTTP, libp2p, etc.) and
    optionally x402 payment negotiation.

    Quick start
    -----------
    from discovery.http_discovery import DiscoveryFactory
    from interfaces.iagent_client import AgentClient

    discovery = DiscoveryFactory.create()
    client    = AgentClient(discovery)

    # Discover-and-call in one step:
    resp = await client.call_capability("weather_forecast", {"city": "NYC"})

    # Or call a specific agent:
    resp = await client.call("sentrix://agent/0xABC", "weather_forecast", {"city": "NYC"})

    # With x402 auto-payment:
    from addons.x402.client import MockWalletProvider
    client = AgentClient(discovery, x402_wallet=MockWalletProvider(), auto_pay=True)
    resp = await client.call_capability("premium_analysis", {"query": "..."})
    """

    # ── Lookup ────────────────────────────────────────────────────────────

    @abstractmethod
    async def find(self, capability: str) -> Optional[DiscoveryEntry]:
        """
        Find the best healthy agent that exposes `capability`.
        Returns None if no healthy agent is registered for this capability.
        """
        ...

    @abstractmethod
    async def find_all(self, capability: str) -> List[DiscoveryEntry]:
        """
        Return all healthy agents that expose `capability`.
        """
        ...

    @abstractmethod
    async def find_by_id(self, agent_id: str) -> Optional[DiscoveryEntry]:
        """
        Look up a specific agent by its agent_id.
        Returns None if not found in the discovery layer.
        """
        ...

    # ── Interaction ───────────────────────────────────────────────────────

    @abstractmethod
    async def call(
        self,
        agent_id: str,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
        timeout_ms: int = 30_000,
    ) -> AgentResponse:
        """
        Call a specific agent by its agent_id.

        Looks up the agent's network endpoint via the discovery layer,
        builds an AgentRequest, and dispatches it over the transport.

        Args:
            agent_id:    The target agent's Sentrix ID (e.g. sentrix://agent/0xABC…)
            capability:  The capability to invoke on the target agent.
            payload:     JSON-serialisable dict passed as the request payload.
            caller_id:   Identity of the calling agent (default: "anonymous").
            timeout_ms:  Request timeout in milliseconds.

        Returns:
            AgentResponse — status is one of: "success", "error", "payment_required"
        """
        ...

    @abstractmethod
    async def call_capability(
        self,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
        timeout_ms: int = 30_000,
    ) -> AgentResponse:
        """
        Discover the best agent for `capability` then call it in one step.

        Equivalent to:
            entry = await client.find(capability)
            return await client.call_entry(entry, capability, payload)

        Returns an error AgentResponse if no healthy agent is found.
        """
        ...

    @abstractmethod
    async def call_entry(
        self,
        entry: DiscoveryEntry,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
        timeout_ms: int = 30_000,
    ) -> AgentResponse:
        """
        Call an agent using a DiscoveryEntry you already have.

        Skips the lookup step — useful when you want to pin a specific
        endpoint or when you retrieved the entry yourself.
        """
        ...

    # ── Mesh protocols ────────────────────────────────────────────────────

    @abstractmethod
    async def ping(
        self,
        agent_id: str,
        *,
        timeout_ms: int = 5_000,
    ) -> "HeartbeatResponse":
        """
        Send a heartbeat ping to an agent and return its response.

        Uses the "__heartbeat" reserved capability. Returns a HeartbeatResponse
        with status="unhealthy" if the agent is unreachable.
        """
        ...

    @abstractmethod
    async def connect(
        self,
        entry: "DiscoveryEntry",
        *,
        timeout_ms: int = 10_000,
    ) -> "AgentSession":
        """
        Establish a connection to a remote agent via a two-step handshake:

          1. Heartbeat ping  — confirms the agent is alive and returns health status.
          2. Capability exchange — asks the agent for its current capability list and
             full ANR record, verifying that the discovery-layer advertisement is still
             accurate before the first call is made.

        Returns an AgentSession that caches the handshake result and provides
        call / ping / refresh_capabilities methods for subsequent interactions.

        Typical usage
        -------------
        entry   = await client.find("weather_forecast")
        session = await client.connect(entry)
        if session.handshake.supports("weather_forecast"):
            resp = await session.call("weather_forecast", {"city": "NYC"})
        """
        ...

    @abstractmethod
    async def gossip_announce(
        self,
        entry: "DiscoveryEntry",
        *,
        ttl: int = 3,
    ) -> None:
        """
        Broadcast a capability announcement to all connected peers.

        Call this after registering a new agent to propagate its capabilities
        across the mesh without waiting for the discovery layer to poll.
        """
        ...

    @abstractmethod
    async def gossip_query(
        self,
        capability: str,
        *,
        ttl: int = 3,
        timeout_ms: int = 5_000,
    ) -> List["DiscoveryEntry"]:
        """
        Broadcast a capability query across the gossip mesh and collect responses.

        Returns entries received within timeout_ms. Useful when the local
        discovery cache is empty or stale.
        """
        ...

    # ── Streaming ─────────────────────────────────────────────────────────

    @abstractmethod
    def stream(
        self,
        agent_id: str,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Stream a capability call to a specific agent by agent_id.

        Connects to ``POST {agent_endpoint}/invoke/stream`` and yields
        ``StreamChunk`` objects as the remote agent produces output, followed
        by a single ``StreamEnd`` frame.

        Args:
            agent_id:   The target agent's Sentrix ID.
            capability: The capability to invoke.
            payload:    JSON-serialisable dict passed as the request payload.
            caller_id:  Identity of the calling agent (default: "anonymous").

        Yields:
            StreamChunk — incremental output (type="chunk").
            StreamEnd   — terminal frame (type="end"); may carry ``error``.
        """
        ...

    @abstractmethod
    def stream_capability(
        self,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Discover the best agent for *capability* then stream it in one step.

        Equivalent to:
            entry = await client.find(capability)
            async for event in client.stream_entry(entry, capability, payload):
                yield event
        """
        ...

    @abstractmethod
    def stream_entry(
        self,
        entry: "DiscoveryEntry",
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "anonymous",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Stream a capability call to an agent using a DiscoveryEntry you already have.

        Skips the lookup step. Useful when you want to pin a specific endpoint.
        """
        ...


# ── AgentClient — HTTP transport implementation ───────────────────────────────

class AgentClient(IAgentClient):
    """
    HTTP-transport implementation of IAgentClient.

    Dispatches AgentRequests as JSON POST to `{protocol}://{host}:{port}/invoke`
    and deserialises the JSON AgentResponse.

    x402 support is built-in — pass `x402_wallet` to enable auto-payment.

    Args:
        discovery:    IAgentDiscovery backend used for all lookups.
        caller_id:    Default caller identity injected into every request.
        timeout_ms:   Default HTTP timeout (overridable per call).
        x402_wallet:  WalletProvider for automatic x402 payment handling.
                      If None (default), payment_required responses are returned as-is.
        auto_pay:     If True, pays x402 challenges without confirmation.
                      If False (default), calls on_payment_required() first.
    """

    def __init__(
        self,
        discovery: IAgentDiscovery,
        *,
        caller_id: str = "anonymous",
        timeout_ms: int = 30_000,
        x402_wallet=None,
        auto_pay: bool = False,
    ):
        self._discovery = discovery
        self._caller_id = caller_id
        self._timeout_ms = timeout_ms
        # x402 is lazy-initialised to avoid import cost when not used
        self._x402_wallet = x402_wallet
        self._auto_pay = auto_pay
        self._x402_client = None
        if x402_wallet is not None:
            from addons.x402.client import X402Client
            self._x402_client = X402Client(wallet=x402_wallet, auto_pay=auto_pay)

    # ── lookup ────────────────────────────────────────────────────────────

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
        for entry in all_entries:
            if entry.agent_id == agent_id:
                return entry
        return None

    # ── interaction ───────────────────────────────────────────────────────

    async def call(
        self,
        agent_id: str,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
        timeout_ms: int = 0,
    ) -> AgentResponse:
        entry = await self.find_by_id(agent_id)
        if entry is None:
            return AgentResponse.error(
                str(uuid.uuid4()),
                f"Agent not found in discovery: {agent_id}",
            )
        return await self.call_entry(
            entry, capability, payload,
            caller_id=caller_id or self._caller_id,
            timeout_ms=timeout_ms or self._timeout_ms,
        )

    async def call_capability(
        self,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
        timeout_ms: int = 0,
    ) -> AgentResponse:
        entry = await self.find(capability)
        if entry is None:
            return AgentResponse.error(
                str(uuid.uuid4()),
                f"No healthy agent found for capability: '{capability}'",
            )
        return await self.call_entry(
            entry, capability, payload,
            caller_id=caller_id or self._caller_id,
            timeout_ms=timeout_ms or self._timeout_ms,
        )

    async def call_entry(
        self,
        entry: DiscoveryEntry,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
        timeout_ms: int = 0,
    ) -> AgentResponse:
        req = AgentRequest(
            request_id=str(uuid.uuid4()),
            from_id=caller_id or self._caller_id,
            capability=capability,
            payload=payload,
            timestamp=int(time.time() * 1000),
        )
        return await self._dispatch(entry, req, timeout_ms or self._timeout_ms)

    # ── mesh protocol implementations ──────────────────────────────────────

    async def ping(
        self,
        agent_id: str,
        *,
        timeout_ms: int = 5_000,
    ) -> "HeartbeatResponse":
        from .iagent_mesh import HeartbeatRequest, HeartbeatResponse
        req_payload = HeartbeatRequest(sender_id=self._caller_id).to_dict()
        resp = await self.call(
            agent_id, "__heartbeat", req_payload,
            caller_id=self._caller_id, timeout_ms=timeout_ms,
        )
        if resp.status == "success" and resp.result:
            try:
                return HeartbeatResponse.from_dict(resp.result)
            except Exception:
                pass
        return HeartbeatResponse(agent_id=agent_id, status="unhealthy")

    async def connect(
        self,
        entry: "DiscoveryEntry",
        *,
        timeout_ms: int = 10_000,
    ) -> "AgentSession":
        """
        Handshake: heartbeat + capability exchange → AgentSession.

        Step 1 — heartbeat ping (liveness + health).
        Step 2 — capability exchange (verify current capabilities match discovery).
        Both steps share the same timeout budget.
        """
        from .iagent_mesh import HandshakeResult, AgentSession
        import time as _time

        t0 = _time.monotonic()

        # Step 1: heartbeat
        hb = await self.ping(entry.agent_id, timeout_ms=timeout_ms)

        # Step 2: capability exchange (only if agent is reachable)
        cap_resp = await self._exchange_capabilities(entry, timeout_ms=timeout_ms)

        latency_ms = int((_time.monotonic() - t0) * 1000)
        handshake = HandshakeResult(
            agent_id=entry.agent_id,
            health_status=hb.status,
            capabilities=cap_resp.capabilities,
            latency_ms=latency_ms,
            anr=cap_resp.anr,
            version=hb.version,
        )
        return AgentSession(entry=entry, handshake=handshake, _client=self)

    async def _exchange_capabilities(
        self,
        entry: "DiscoveryEntry",
        *,
        timeout_ms: int = 10_000,
    ) -> "CapabilityExchangeResponse":
        """Internal: run capability exchange against a known DiscoveryEntry."""
        from .iagent_mesh import CapabilityExchangeRequest, CapabilityExchangeResponse
        req_payload = CapabilityExchangeRequest(
            sender_id=self._caller_id, include_anr=True,
        ).to_dict()
        resp = await self.call_entry(
            entry, "__capabilities", req_payload,
            timeout_ms=timeout_ms,
        )
        if resp.status == "success" and resp.result:
            try:
                return CapabilityExchangeResponse.from_dict(resp.result)
            except Exception:
                pass
        return CapabilityExchangeResponse(agent_id=entry.agent_id, capabilities=[])

    async def gossip_announce(
        self,
        entry: "DiscoveryEntry",
        *,
        ttl: int = 3,
    ) -> None:
        """
        Broadcast an announce message to all peers reachable via discovery.
        Falls back gracefully if no gossip protocol is configured.
        """
        from .iagent_mesh import GossipMessage
        import dataclasses
        msg = GossipMessage(
            type="announce",
            sender_id=self._caller_id,
            ttl=ttl,
            entry=dataclasses.asdict(entry),
        )
        # Fan-out: call "__gossip" on every known peer
        peers = await self._discovery.list_all()
        for peer in peers:
            if peer.agent_id == self._caller_id:
                continue
            try:
                await self.call_entry(
                    peer, "__gossip", msg.to_dict(),
                    caller_id=self._caller_id, timeout_ms=2_000,
                )
            except Exception:
                pass  # best-effort; gossip is fire-and-forget

    async def gossip_query(
        self,
        capability: str,
        *,
        ttl: int = 3,
        timeout_ms: int = 5_000,
    ) -> List["DiscoveryEntry"]:
        from .iagent_mesh import GossipMessage
        from .iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
        import dataclasses
        msg = GossipMessage(
            type="query",
            sender_id=self._caller_id,
            ttl=ttl,
            capability=capability,
        )
        results: List[DiscoveryEntry] = []
        peers = await self._discovery.list_all()
        for peer in peers:
            if peer.agent_id == self._caller_id:
                continue
            try:
                resp = await self.call_entry(
                    peer, "__gossip", msg.to_dict(),
                    caller_id=self._caller_id, timeout_ms=timeout_ms,
                )
                if resp.status == "success" and resp.result:
                    entries_raw = resp.result.get("entries", [])
                    for raw in entries_raw:
                        try:
                            net = raw.get("network", {})
                            results.append(DiscoveryEntry(
                                agent_id=raw["agent_id"],
                                name=raw.get("name", ""),
                                owner=raw.get("owner", "anonymous"),
                                capabilities=raw.get("capabilities", []),
                                network=NetworkInfo(
                                    protocol=net.get("protocol", "http"),
                                    host=net.get("host", "localhost"),
                                    port=net.get("port", 6174),
                                    tls=net.get("tls", False),
                                ),
                                health=HealthStatus(
                                    status=raw.get("health", {}).get("status", "healthy"),
                                    last_heartbeat=raw.get("health", {}).get("last_heartbeat", ""),
                                ),
                                registered_at=raw.get("registered_at", ""),
                            ))
                        except Exception:
                            pass
            except Exception:
                pass
        return results

    # ── streaming ─────────────────────────────────────────────────────────

    async def stream(
        self,
        agent_id: str,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        from .iagent_mesh import StreamEnd
        entry = await self.find_by_id(agent_id)
        if entry is None:
            yield StreamEnd(
                request_id=str(uuid.uuid4()),
                error=f"Agent not found in discovery: {agent_id}",
            )
            return
        async for event in self.stream_entry(entry, capability, payload, caller_id=caller_id or self._caller_id):
            yield event

    async def stream_capability(
        self,
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        from .iagent_mesh import StreamEnd
        entry = await self.find(capability)
        if entry is None:
            yield StreamEnd(
                request_id=str(uuid.uuid4()),
                error=f"No healthy agent found for capability: '{capability}'",
            )
            return
        async for event in self.stream_entry(entry, capability, payload, caller_id=caller_id or self._caller_id):
            yield event

    async def stream_entry(
        self,
        entry: "DiscoveryEntry",
        capability: str,
        payload: Dict[str, Any],
        *,
        caller_id: str = "",
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Connect to POST {agent_endpoint}/invoke/stream and yield SSE events.

        Parses each ``data: <json>`` line and yields StreamChunk / StreamEnd.
        Falls back to the regular /invoke endpoint if /invoke/stream returns
        a non-streaming response.
        """
        import json
        from .iagent_mesh import StreamChunk, StreamEnd

        req = AgentRequest(
            request_id=str(uuid.uuid4()),
            from_id=caller_id or self._caller_id,
            capability=capability,
            payload=payload,
            timestamp=int(time.time() * 1000),
            stream=True,
        )

        url = self._stream_url(entry)
        body = json.dumps(req.to_dict()).encode()
        timeout_s = self._timeout_ms / 1000.0

        try:
            async for event in self._http_stream(url, body, timeout_s):
                yield event
        except Exception as exc:
            yield StreamEnd(request_id=req.request_id, error=str(exc))

    @staticmethod
    def _stream_url(entry: "DiscoveryEntry") -> str:
        scheme = "https" if entry.network.tls else entry.network.protocol
        if scheme not in ("http", "https"):
            scheme = "https" if entry.network.tls else "http"
        return f"{scheme}://{entry.network.host}:{entry.network.port}/invoke/stream"

    @staticmethod
    async def _http_stream(
        url: str,
        body: bytes,
        timeout_s: float,
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Open an SSE connection to *url*, parse ``data:`` lines, and yield
        StreamChunk / StreamEnd objects.

        Prefers httpx (async streaming), falls back to aiohttp.
        """
        import json
        from .iagent_mesh import StreamChunk, StreamEnd

        def _parse_event(line: str):
            raw = line.strip()
            if not raw.startswith("data:"):
                return None
            payload_str = raw[5:].strip()
            if not payload_str:
                return None
            try:
                d = json.loads(payload_str)
            except Exception:
                return None
            event_type = d.get("type", "chunk")
            if event_type == "end":
                return StreamEnd.from_dict(d)
            return StreamChunk.from_dict(d)

        try:
            import httpx
            async with httpx.AsyncClient(timeout=timeout_s) as client:
                async with client.stream(
                    "POST", url, content=body,
                    headers={"Content-Type": "application/json", "Accept": "text/event-stream"},
                ) as response:
                    async for line in response.aiter_lines():
                        event = _parse_event(line)
                        if event is not None:
                            yield event
                            if isinstance(event, StreamEnd):
                                return
            return
        except ImportError:
            pass

        try:
            import aiohttp
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    url, data=body,
                    headers={"Content-Type": "application/json", "Accept": "text/event-stream"},
                    timeout=aiohttp.ClientTimeout(total=timeout_s),
                ) as response:
                    async for line in response.content:
                        decoded = line.decode("utf-8", errors="replace").rstrip()
                        event = _parse_event(decoded)
                        if event is not None:
                            yield event
                            if isinstance(event, StreamEnd):
                                return
            return
        except ImportError:
            pass

        yield StreamEnd(
            request_id="unknown",
            error="No async HTTP backend with streaming support found. "
                  "Install: pip install httpx  or  pip install aiohttp",
        )

    # ── internal transport ────────────────────────────────────────────────

    async def _dispatch(
        self,
        entry: DiscoveryEntry,
        req: AgentRequest,
        timeout_ms: int,
    ) -> AgentResponse:
        """POST AgentRequest JSON to agent endpoint; return deserialized AgentResponse."""
        import json
        url = self._endpoint_url(entry)
        body = json.dumps(req.to_dict()).encode()
        timeout_s = timeout_ms / 1000.0

        raw_response = await self._http_post(url, body, timeout_s)
        response = AgentResponse(**_parse_response(raw_response))

        # x402 auto-payment
        if response.status == "payment_required" and self._x402_client is not None:
            reqs = getattr(response, "payment_requirements", None) or []
            if reqs:
                from addons.x402.types import X402PaymentRequirements
                requirements = X402PaymentRequirements.from_dict(reqs[0])
                if self._auto_pay or await self._x402_client.on_payment_required(requirements, req, response):
                    payment = await self._x402_client.wallet.sign_payment(requirements, req)
                    paid_req = req
                    paid_req.x402 = payment
                    body2 = json.dumps(paid_req.to_dict()).encode()
                    raw_retry = await self._http_post(url, body2, timeout_s)
                    return AgentResponse(**_parse_response(raw_retry))

        return response

    @staticmethod
    def _endpoint_url(entry: DiscoveryEntry) -> str:
        scheme = "https" if entry.network.tls else entry.network.protocol
        if scheme not in ("http", "https"):
            scheme = "https" if entry.network.tls else "http"
        return f"{scheme}://{entry.network.host}:{entry.network.port}/invoke"

    @staticmethod
    async def _http_post(url: str, body: bytes, timeout_s: float) -> bytes:
        """HTTP POST — prefers httpx (async), falls back to aiohttp, then urllib."""
        try:
            import httpx
            async with httpx.AsyncClient(timeout=timeout_s) as client:
                r = await client.post(url, content=body, headers={"Content-Type": "application/json"})
                r.raise_for_status()
                return r.content
        except ImportError:
            pass
        try:
            import aiohttp
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    url, data=body,
                    headers={"Content-Type": "application/json"},
                    timeout=aiohttp.ClientTimeout(total=timeout_s),
                ) as r:
                    r.raise_for_status()
                    return await r.read()
        except ImportError:
            pass
        # Sync fallback via urllib (blocking — acceptable for simple scripts)
        import urllib.request
        http_req = urllib.request.Request(
            url, data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(http_req, timeout=timeout_s) as resp:
            return resp.read()


def _parse_response(raw: bytes) -> dict:
    import json
    d = json.loads(raw)
    # Normalise camelCase → snake_case for AgentResponse fields
    return {
        "request_id":           d.get("requestId", d.get("request_id", "")),
        "status":               d.get("status", "error"),
        "result":               d.get("result"),
        "error_message":        d.get("errorMessage", d.get("error_message")),
        "proof":                d.get("proof"),
        "signature":            d.get("signature"),
        "timestamp":            d.get("timestamp", int(time.time() * 1000)),
        "payment_requirements": d.get("paymentRequirements", d.get("payment_requirements")),
    }
