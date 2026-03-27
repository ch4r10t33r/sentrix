"""
ExampleAgent — starter template.
Replace the capability implementations with your own logic.
"""

from interfaces import IAgent, AgentRequest, AgentResponse
from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
from datetime import datetime, timezone


class ExampleAgent(IAgent):
    # ── ERC-8004 Identity ─────────────────────────────────────────────────
    agent_id     = "borgkit://agent/example"
    owner        = "0xYourWalletAddress"
    metadata_uri = "ipfs://QmYourMetadataHashHere"
    metadata     = {
        "name":        "ExampleAgent",
        "version":     "0.1.0",
        "description": "A starter Borgkit agent",
        "tags":        ["example", "starter"],
    }

    _registry  = None
    _p2p_info  = None

    # ── Capabilities ──────────────────────────────────────────────────────
    def get_capabilities(self):
        return ["echo", "ping"]

    # ── Request handling ──────────────────────────────────────────────────
    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        if not await self.check_permission(req.from_id, req.capability):
            return AgentResponse.error(req.request_id, "Permission denied")

        match req.capability:
            case "echo":
                return AgentResponse.success(req.request_id, {"echo": req.payload})
            case "ping":
                return AgentResponse.success(req.request_id, {"pong": True, "agentId": self.agent_id})
            case _:
                return AgentResponse.error(req.request_id, f'Unknown capability: "{req.capability}"')

    # ── Discovery ─────────────────────────────────────────────────────────
    async def register_discovery(self) -> None:
        import os
        from discovery.http_discovery import DiscoveryFactory

        discovery_type = os.environ.get('BORGKIT_DISCOVERY_TYPE', 'libp2p')
        registry = await DiscoveryFactory.create(discovery_type)
        self._registry = registry
        self._p2p_info = None

        # Capture libp2p node info (peerId + multiaddr) if in P2P mode
        if hasattr(registry, 'get_node_info'):
            self._p2p_info = await registry.get_node_info()

        await registry.register(self._build_entry())
        print("[ExampleAgent] registered with discovery layer")

    async def unregister_discovery(self) -> None:
        registry = getattr(self, '_registry', None)
        if registry:
            await registry.unregister(self.agent_id)
        else:
            from discovery.local_discovery import LocalDiscovery
            await LocalDiscovery.get_instance().unregister(self.agent_id)

    def _build_entry(self) -> 'DiscoveryEntry':
        import os
        from datetime import datetime, timezone
        host = os.environ.get('BORGKIT_HOST', 'localhost')
        port = int(os.environ.get('BORGKIT_PORT', '6174'))
        tls  = os.environ.get('BORGKIT_TLS', 'false').lower() == 'true'

        p2p  = getattr(self, '_p2p_info', None)
        peer_id  = p2p.get('peer_id')  if p2p else None
        maddr    = p2p.get('multiaddr') if p2p else None

        if peer_id and not maddr:
            maddr = f"/ip4/{host}/tcp/{port}/p2p/{peer_id}"

        net = NetworkInfo(
            protocol='libp2p' if peer_id else 'http',
            host=host,
            port=port,
            tls=tls,
            peer_id=peer_id or '',
            multiaddr=maddr or '',
        )
        return DiscoveryEntry(
            agent_id=self.agent_id,
            name="ExampleAgent",
            owner=self.owner,
            capabilities=self.get_capabilities(),
            network=net,
            health=HealthStatus(status="healthy", last_heartbeat=datetime.now(timezone.utc).isoformat()),
            registered_at=datetime.now(timezone.utc).isoformat(),
        )


# ── Entry point ───────────────────────────────────────────────────────────────
#
# Run directly:
#   python agents/example_agent.py
#   BORGKIT_PORT=9090 python agents/example_agent.py
#
# Or via borgkit-cli:
#   borgkit run ExampleAgent --port 6174
#
if __name__ == "__main__":
    import asyncio
    import os
    from server import serve

    port = int(os.environ.get("BORGKIT_PORT", "6174"))

    agent = ExampleAgent()
    asyncio.run(serve(agent, port=port))
