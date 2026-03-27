"""
Borgkit Mesh Protocol — Heartbeat, Capability Exchange, Gossip, and Streaming
──────────────────────────────────────────────────────────────────────────────
Defines the message types and interfaces for the four built-in agent-to-agent
protocols that every Borgkit agent understands:

  1. Heartbeat          — liveness ping with status payload
  2. Capability Exchange — direct capability query (bypasses discovery layer)
  3. Gossip             — capability announcements fan-out across the mesh
  4. Streaming          — incremental token / result delivery (SSE or libp2p)

These protocols ride on top of the standard AgentRequest / AgentResponse
envelope using reserved capability names:

  "__heartbeat"    → HeartbeatRequest / HeartbeatResponse
  "__capabilities" → CapabilityExchangeRequest / CapabilityExchangeResponse
  "__gossip"       → GossipMessage (fire-and-forget; no structured response)

Streaming uses a dedicated endpoint (POST /invoke/stream) and emits
Server-Sent Events with StreamChunk frames, terminated by a StreamEnd frame.

Agents that implement IAgent via WrappedAgent get default implementations
of all three mesh protocols for free. Override the methods for custom behaviour.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any, AsyncGenerator, Callable, Awaitable, Dict, List, Optional, Union
import time


# ── Heartbeat ─────────────────────────────────────────────────────────────────

@dataclass
class HeartbeatRequest:
    """Sent by one agent to check liveness and status of another."""
    sender_id:  str
    timestamp:  int = field(default_factory=lambda: int(time.time() * 1000))
    nonce:      str = ""        # optional correlation token

    def to_dict(self) -> dict:
        return {"senderId": self.sender_id, "timestamp": self.timestamp, "nonce": self.nonce}

    @staticmethod
    def from_dict(d: dict) -> "HeartbeatRequest":
        return HeartbeatRequest(
            sender_id=d.get("senderId", d.get("sender_id", "unknown")),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
            nonce=d.get("nonce", ""),
        )


@dataclass
class HeartbeatResponse:
    """Returned in response to a HeartbeatRequest."""
    agent_id:          str
    status:            str        # "healthy" | "degraded" | "unhealthy"
    timestamp:         int = field(default_factory=lambda: int(time.time() * 1000))
    capabilities_count: int = 0
    uptime_ms:         int = 0
    version:           str = ""
    nonce:             str = ""   # echoes HeartbeatRequest.nonce

    def to_dict(self) -> dict:
        return {
            "agentId":          self.agent_id,
            "status":           self.status,
            "timestamp":        self.timestamp,
            "capabilitiesCount": self.capabilities_count,
            "uptimeMs":         self.uptime_ms,
            "version":          self.version,
            "nonce":            self.nonce,
        }

    @staticmethod
    def from_dict(d: dict) -> "HeartbeatResponse":
        return HeartbeatResponse(
            agent_id=d.get("agentId", d.get("agent_id", "")),
            status=d.get("status", "unknown"),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
            capabilities_count=d.get("capabilitiesCount", d.get("capabilities_count", 0)),
            uptime_ms=d.get("uptimeMs", d.get("uptime_ms", 0)),
            version=d.get("version", ""),
            nonce=d.get("nonce", ""),
        )


# ── Capability Exchange ───────────────────────────────────────────────────────

@dataclass
class CapabilityExchangeRequest:
    """Ask an agent to describe its current capabilities directly (bypasses discovery)."""
    sender_id:   str
    timestamp:   int = field(default_factory=lambda: int(time.time() * 1000))
    include_anr: bool = True    # if True, response includes full DiscoveryEntry

    def to_dict(self) -> dict:
        return {"senderId": self.sender_id, "timestamp": self.timestamp, "includeAnr": self.include_anr}

    @staticmethod
    def from_dict(d: dict) -> "CapabilityExchangeRequest":
        return CapabilityExchangeRequest(
            sender_id=d.get("senderId", d.get("sender_id", "unknown")),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
            include_anr=d.get("includeAnr", d.get("include_anr", True)),
        )


@dataclass
class CapabilityExchangeResponse:
    """Response to a CapabilityExchangeRequest."""
    agent_id:     str
    capabilities: List[str]
    timestamp:    int = field(default_factory=lambda: int(time.time() * 1000))
    # Full ANR record — present when CapabilityExchangeRequest.include_anr is True
    anr:          Optional[Dict[str, Any]] = None

    def to_dict(self) -> dict:
        return {
            "agentId":      self.agent_id,
            "capabilities": self.capabilities,
            "timestamp":    self.timestamp,
            "anr":          self.anr,
        }

    @staticmethod
    def from_dict(d: dict) -> "CapabilityExchangeResponse":
        return CapabilityExchangeResponse(
            agent_id=d.get("agentId", d.get("agent_id", "")),
            capabilities=d.get("capabilities", []),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
            anr=d.get("anr"),
        )


# ── Gossip ────────────────────────────────────────────────────────────────────

@dataclass
class GossipMessage:
    """
    A capability announcement or revocation propagated peer-to-peer.

    Agents forward gossip messages to their peers with ttl decremented by 1.
    When ttl reaches 0 the message is no longer forwarded (prevents loops).
    `seen_by` is used as a bloom filter to avoid re-processing at known nodes.

    Types:
      "announce"  — agent is online and advertising capabilities
      "revoke"    — agent is going offline or revoking capabilities
      "heartbeat" — lightweight liveness ping propagated to neighbours
      "query"     — ask the mesh for agents with a given capability
    """
    type:       str                   # "announce" | "revoke" | "heartbeat" | "query"
    sender_id:  str
    timestamp:  int = field(default_factory=lambda: int(time.time() * 1000))
    ttl:        int = 3               # max hops before message is dropped
    seen_by:    List[str] = field(default_factory=list)

    # Payload fields (presence depends on type)
    entry:       Optional[Dict[str, Any]] = None   # DiscoveryEntry dict (announce/revoke)
    capability:  Optional[str] = None              # capability name (query)
    nonce:       str = ""

    def to_dict(self) -> dict:
        return {
            "type":       self.type,
            "senderId":   self.sender_id,
            "timestamp":  self.timestamp,
            "ttl":        self.ttl,
            "seenBy":     self.seen_by,
            "entry":      self.entry,
            "capability": self.capability,
            "nonce":      self.nonce,
        }

    @staticmethod
    def from_dict(d: dict) -> "GossipMessage":
        return GossipMessage(
            type=d.get("type", "announce"),
            sender_id=d.get("senderId", d.get("sender_id", "")),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
            ttl=d.get("ttl", 3),
            seen_by=d.get("seenBy", d.get("seen_by", [])),
            entry=d.get("entry"),
            capability=d.get("capability"),
            nonce=d.get("nonce", ""),
        )

    def forwarded_by(self, agent_id: str) -> "GossipMessage":
        """Return a copy of this message with ttl decremented and agent_id appended to seen_by."""
        import copy
        msg = copy.copy(self)
        msg.ttl = self.ttl - 1
        msg.seen_by = list(self.seen_by) + [agent_id]
        return msg

    @property
    def should_forward(self) -> bool:
        """True if this message still has hops remaining."""
        return self.ttl > 0


# ── IGossipProtocol ───────────────────────────────────────────────────────────

GossipHandler = Callable[["GossipMessage"], Awaitable[None]]


class IGossipProtocol(ABC):
    """
    Interface for gossip-based capability propagation.

    Implement this to build a gossip transport (TCP, libp2p pubsub, WebSocket, etc.)
    The default GossipDiscovery uses HTTP fan-out for simplicity.
    """

    @abstractmethod
    async def broadcast(self, message: GossipMessage) -> None:
        """Fan out a gossip message to all currently connected peers."""
        ...

    @abstractmethod
    async def receive(self, message: GossipMessage) -> None:
        """
        Process an incoming gossip message from a peer.

        Implementations should:
          1. Ignore messages already seen (check seen_by / nonce)
          2. Apply the message to local state (update registry)
          3. Forward with ttl-1 if should_forward
        """
        ...

    @abstractmethod
    def subscribe(self, handler: GossipHandler) -> None:
        """Register a callback invoked for every incoming gossip message."""
        ...

    @abstractmethod
    def peers(self) -> List[str]:
        """Return agent IDs of currently connected peers."""
        ...

    @abstractmethod
    async def add_peer(self, agent_id: str, endpoint: str) -> None:
        """Connect to a new peer by agent_id and HTTP endpoint."""
        ...

    @abstractmethod
    async def remove_peer(self, agent_id: str) -> None:
        """Disconnect from a peer."""
        ...


# ── Handshake ─────────────────────────────────────────────────────────────────

@dataclass
class HandshakeResult:
    """
    Result of the connection handshake performed by AgentClient.connect().

    Produced by a single round-trip that combines a heartbeat ping (liveness +
    health) with a capability exchange (what the agent currently supports).
    Capability exchange is intentionally part of the handshake — it verifies
    that the discovered agent still advertises the capabilities you need before
    you commit to calling it.
    """
    agent_id:      str
    health_status: str          # "healthy" | "degraded" | "unhealthy"
    capabilities:  List[str]
    latency_ms:    int          # round-trip time of the handshake
    connected_at:  int = field(default_factory=lambda: int(time.time() * 1000))
    # Full ANR record if the agent returned include_anr=True
    anr:           Optional[Any] = None  # DiscoveryEntry dict
    version:       str = ""

    def supports(self, capability: str) -> bool:
        """True if this agent declared the given capability during handshake."""
        return capability in self.capabilities


@dataclass
class AgentSession:
    """
    An active connection to a remote agent, established by AgentClient.connect().

    Holds the handshake result (capabilities + health snapshot) and provides
    call/ping methods that reuse the discovered endpoint without re-querying
    the discovery layer on every request.

    Typical usage
    -------------
    session = await client.connect(entry)
    if not session.handshake.supports("weather_forecast"):
        raise ValueError("Agent no longer advertises weather_forecast")
    resp = await session.call("weather_forecast", {"city": "NYC"})

    # Re-validate after a period of inactivity:
    fresh = await session.refresh_capabilities()
    """
    entry:     Any          # DiscoveryEntry — typed as Any to avoid circular import
    handshake: HandshakeResult
    _client:   Any          # AgentClient — resolved at runtime

    @property
    def agent_id(self) -> str:
        return self.handshake.agent_id

    @property
    def capabilities(self) -> List[str]:
        return self.handshake.capabilities

    @property
    def is_healthy(self) -> bool:
        return self.handshake.health_status == "healthy"

    async def call(
        self,
        capability: str,
        payload: Dict[str, Any],
        *,
        timeout_ms: int = 30_000,
    ) -> Any:  # AgentResponse
        """Call a capability on this agent using the established session."""
        return await self._client.call_entry(
            self.entry, capability, payload,
            timeout_ms=timeout_ms,
        )

    async def ping(self, *, timeout_ms: int = 5_000) -> HeartbeatResponse:
        """Re-check liveness of this agent."""
        return await self._client.ping(self.agent_id, timeout_ms=timeout_ms)

    async def refresh_capabilities(self) -> CapabilityExchangeResponse:
        """
        Re-run the capability exchange for this agent.

        Use this after a period of inactivity to verify the agent still
        supports the capabilities cached in the handshake.
        """
        return await self._client._exchange_capabilities(
            self.entry, timeout_ms=10_000,
        )

    async def stream(
        self,
        capability: str,
        payload: Dict[str, Any],
    ) -> "AsyncGenerator[StreamChunk | StreamEnd, None]":
        """
        Stream a capability call on this agent using the established session.

        Yields StreamChunk objects as they arrive, followed by a final StreamEnd.
        Uses the ``POST /invoke/stream`` SSE endpoint on the remote agent.

        Example::

            async for chunk in session.stream("summarise", {"text": long_text}):
                if chunk.type == "chunk":
                    print(chunk.delta, end="", flush=True)
                elif chunk.type == "end":
                    break
        """
        async for event in self._client.stream_entry(self.entry, capability, payload):
            yield event

    async def close(self) -> None:
        """Signal to the remote agent that this session is ending (best-effort)."""
        try:
            await self._client.call_entry(
                self.entry, "__disconnect",
                {"sessionAgentId": self.agent_id},
                timeout_ms=2_000,
            )
        except Exception:
            pass  # close is best-effort


# ── Streaming ─────────────────────────────────────────────────────────────────

@dataclass
class StreamChunk:
    """
    A single incremental chunk delivered during a streaming capability call.

    Sent as an SSE frame on POST /invoke/stream while the agent is still
    producing output. For LLM agents, ``delta`` carries the token text.
    For search or structured agents, ``result`` carries a partial structured
    result instead (or in addition).

    Frame wire format (SSE):
        data: {"type":"chunk","requestId":"…","delta":"…","sequence":N,"timestamp":T}
    """
    request_id: str
    type:       str = "chunk"              # always "chunk"
    delta:      str = ""                   # LLM token text / text delta
    result:     Optional[Any] = None       # structured partial result (optional)
    sequence:   int = 0                    # monotonically increasing per request
    timestamp:  int = field(default_factory=lambda: int(time.time() * 1000))

    def to_dict(self) -> dict:
        d: dict = {
            "type":      self.type,
            "requestId": self.request_id,
            "delta":     self.delta,
            "sequence":  self.sequence,
            "timestamp": self.timestamp,
        }
        if self.result is not None:
            d["result"] = self.result
        return d

    @staticmethod
    def from_dict(d: dict) -> "StreamChunk":
        return StreamChunk(
            request_id=d.get("requestId", d.get("request_id", "")),
            type=d.get("type", "chunk"),
            delta=d.get("delta", ""),
            result=d.get("result"),
            sequence=d.get("sequence", 0),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
        )


@dataclass
class StreamEnd:
    """
    Terminal frame of a streaming capability call.

    Sent as the last SSE frame on POST /invoke/stream. ``final_result``
    carries the complete assembled result (for callers that only want the
    final value). ``error`` is set on abnormal termination.

    Frame wire format (SSE):
        data: {"type":"end","requestId":"…","finalResult":{…},"sequence":N,"timestamp":T}
    """
    request_id:   str
    type:         str = "end"             # always "end"
    final_result: Optional[Any] = None    # complete assembled result
    error:        Optional[str] = None    # set on error / cancellation
    sequence:     int = 0
    timestamp:    int = field(default_factory=lambda: int(time.time() * 1000))

    def to_dict(self) -> dict:
        d: dict = {
            "type":      self.type,
            "requestId": self.request_id,
            "sequence":  self.sequence,
            "timestamp": self.timestamp,
        }
        if self.final_result is not None:
            d["finalResult"] = self.final_result
        if self.error is not None:
            d["error"] = self.error
        return d

    @staticmethod
    def from_dict(d: dict) -> "StreamEnd":
        return StreamEnd(
            request_id=d.get("requestId", d.get("request_id", "")),
            type=d.get("type", "end"),
            final_result=d.get("finalResult", d.get("final_result")),
            error=d.get("error"),
            sequence=d.get("sequence", 0),
            timestamp=d.get("timestamp", int(time.time() * 1000)),
        )
