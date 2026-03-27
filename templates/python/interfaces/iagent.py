"""
Borgkit agent interface.
Every Borgkit agent must implement this abstract base class.

Identity note
─────────────
``agent_id`` is required; ``owner`` is optional.
ERC-8004 on-chain registration is not required — a local secp256k1 key is
sufficient for signing ANR records and P2P discovery.
See identity.provider.LocalKeystoreIdentity for the default no-wallet option.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, AsyncGenerator, List, Optional, Union
from .agent_request import AgentRequest
from .agent_response import AgentResponse

if TYPE_CHECKING:
    from .iagent_discovery import DiscoveryEntry
    from .iagent_mesh import (
        HeartbeatRequest, HeartbeatResponse,
        CapabilityExchangeRequest, CapabilityExchangeResponse,
        GossipMessage, StreamChunk, StreamEnd,
    )


@dataclass
class ResourceRequirements:
    min_memory_mb:  Optional[int]   = None
    min_cpu_cores:  Optional[float] = None
    storage_gb:     Optional[float] = None


@dataclass
class AgentMetadata:
    name:                  str
    version:               str
    description:           Optional[str]                  = None
    author:                Optional[str]                  = None
    license:               Optional[str]                  = None
    repository:            Optional[str]                  = None
    tags:                  List[str]                      = field(default_factory=list)
    resource_requirements: Optional[ResourceRequirements] = None


class IAgent(ABC):
    # ── Identity ───────────────────────────────────────────────────────────
    agent_id: str                            # e.g. "borgkit://agent/0xABC..."
    owner: str              = "anonymous"    # Wallet address or arbitrary identifier.
                                             # Required for ERC-8004; optional otherwise.
    metadata_uri: Optional[str]          = None
    metadata:     Optional[AgentMetadata] = None

    # ── Capabilities ───────────────────────────────────────────────────────
    @abstractmethod
    def get_capabilities(self) -> List[str]:
        """Return list of capability names this agent exposes."""
        ...

    # ── Request handling ───────────────────────────────────────────────────
    @abstractmethod
    async def handle_request(self, request: AgentRequest) -> AgentResponse:
        """Primary dispatch — all inbound calls arrive here."""
        ...

    async def stream_request(
        self,
        request: AgentRequest,
    ) -> "AsyncGenerator[Union[StreamChunk, StreamEnd], None]":
        """
        Streaming variant of handle_request.

        Yields ``StreamChunk`` objects as incremental output is produced,
        then a single ``StreamEnd`` to signal completion.

        Default implementation falls back to ``handle_request`` and emits
        the full result as one chunk followed by a StreamEnd — so all agents
        support the streaming endpoint out of the box without any changes.

        Override in framework plugins that can produce genuine token streams
        (e.g. LangGraph with streaming LLMs, OpenAI Agents SDK streaming, etc.)

        Example override::

            async def stream_request(self, request):
                sequence = 0
                async for token in my_llm.stream(request.payload["prompt"]):
                    yield StreamChunk(
                        request_id=request.request_id,
                        delta=token,
                        sequence=sequence,
                    )
                    sequence += 1
                yield StreamEnd(request_id=request.request_id, sequence=sequence)
        """
        from .iagent_mesh import StreamChunk, StreamEnd
        resp = await self.handle_request(request)
        content = ""
        if resp.result:
            content = resp.result.get("content", "") if isinstance(resp.result, dict) else str(resp.result)
        if resp.error_message:
            yield StreamEnd(
                request_id=request.request_id,
                error=resp.error_message,
                sequence=0,
            )
            return
        if content:
            yield StreamChunk(
                request_id=request.request_id,
                delta=content,
                result=resp.result,
                sequence=0,
            )
        yield StreamEnd(
            request_id=request.request_id,
            final_result=resp.result,
            sequence=1 if content else 0,
        )

    async def pre_process(self, request: AgentRequest) -> None:
        """Optional hook: auth, rate-limit, logging. Override as needed."""
        pass

    async def post_process(self, response: AgentResponse) -> None:
        """Optional hook: audit log, billing. Override as needed."""
        pass

    # ── Discovery (optional) ───────────────────────────────────────────────
    async def register_discovery(self) -> None:
        """Announce this agent to the discovery layer."""
        pass

    async def unregister_discovery(self) -> None:
        """Gracefully withdraw from the discovery layer."""
        pass

    # ── Delegation / permissions (optional) ────────────────────────────────
    async def check_permission(self, caller: str, capability: str) -> bool:
        """Return True if `caller` is permitted to invoke `capability`."""
        return True  # open by default; override for production

    # ── Mesh protocols (heartbeat / capability exchange / gossip) ──────────

    async def handle_heartbeat(self, req: "HeartbeatRequest") -> "HeartbeatResponse":
        """
        Respond to a heartbeat ping from another agent.

        Default implementation returns status="healthy" with capability count.
        Override to add custom health checks (DB reachability, model status, etc.)
        """
        from .iagent_mesh import HeartbeatResponse
        return HeartbeatResponse(
            agent_id=self.agent_id,
            status="healthy",
            capabilities_count=len(self.get_capabilities()),
        )

    async def handle_capability_exchange(
        self, req: "CapabilityExchangeRequest"
    ) -> "CapabilityExchangeResponse":
        """
        Respond to a direct capability query from another agent.

        Default implementation returns capabilities and the full ANR record.
        """
        from .iagent_mesh import CapabilityExchangeResponse
        anr_dict = None
        if req.include_anr:
            try:
                import dataclasses
                anr_dict = dataclasses.asdict(self.get_anr())
            except Exception:
                pass
        return CapabilityExchangeResponse(
            agent_id=self.agent_id,
            capabilities=self.get_capabilities(),
            anr=anr_dict,
        )

    async def handle_gossip(self, msg: "GossipMessage") -> None:
        """
        Process an incoming gossip message.

        Default: no-op. Override to act on announce/revoke/query gossip
        or plug in a GossipDiscovery backend.
        """
        pass

    # ── ANR / Identity exposure ────────────────────────────────────────────
    @abstractmethod
    def get_anr(self) -> "DiscoveryEntry":
        """
        Return the full ANR (Agent Network Record) for this agent.

        The ANR is the authoritative self-description of the agent on the mesh:
        its identity, capabilities, network endpoint, and health status.
        Callers can use this to inspect a live agent without querying the
        discovery layer.
        """
        ...

    def get_peer_id(self) -> Optional[str]:
        """
        Return the libp2p PeerId derived from this agent's secp256k1 ANR key.

        The PeerId is derived from the same key used to sign ANR records —
        one keypair, one identity across both the ANR layer and the P2P
        transport layer.

        Returns None for anonymous agents (no signing key configured).
        """
        return None

    # ── Signing (optional) ─────────────────────────────────────────────────
    async def sign_message(self, message: str) -> str:
        """
        Sign a message with this agent's secp256k1 private key.

        Uses (in priority order):
          1. self._identity.sign_bytes()  if an IdentityProvider is attached
          2. BORGKIT_AGENT_KEY env var    (hex private key)

        Raises RuntimeError if no signing key is configured.
        """
        import os

        # 1. Use attached identity provider
        identity = getattr(self, '_identity', None)
        if identity is not None and hasattr(identity, 'sign_bytes'):
            sig = identity.sign_bytes(message.encode('utf-8'))
            if sig is not None:
                return sig

        # 2. Fall back to BORGKIT_AGENT_KEY env var
        raw_key = os.environ.get('BORGKIT_AGENT_KEY', '')
        if raw_key:
            key_hex = raw_key.lstrip('0x')
            key_bytes = bytes.fromhex(key_hex)
            try:
                from eth_keys import keys as eth_keys
                pk = eth_keys.PrivateKey(key_bytes)
                msg_bytes = message.encode('utf-8')
                # Ethereum personal sign prefix
                prefix = f"\x19Ethereum Signed Message:\n{len(msg_bytes)}".encode()
                from eth_hash.auto import keccak
                msg_hash = keccak(prefix + msg_bytes)
                return pk.sign_msg_hash(msg_hash).to_hex()
            except ImportError:
                pass
            try:
                import coincurve, hashlib
                sk = coincurve.PrivateKey(key_bytes)
                msg_bytes = message.encode('utf-8')
                prefix = f"\x19Ethereum Signed Message:\n{len(msg_bytes)}".encode()
                msg_hash = hashlib.sha3_256(prefix + msg_bytes).digest()  # keccak256 via sha3
                sig = sk.sign_recoverable(msg_hash, hasher=None)
                return '0x' + sig.hex()
            except ImportError:
                pass
            raise RuntimeError(
                "sign_message: eth-keys or coincurve required. "
                "Install: pip install eth-keys  or  pip install coincurve"
            )

        raise RuntimeError(
            "sign_message: no signing key configured. "
            "Attach an IdentityProvider via self._identity = provider, "
            "or set BORGKIT_AGENT_KEY=<hex-private-key> env var."
        )
