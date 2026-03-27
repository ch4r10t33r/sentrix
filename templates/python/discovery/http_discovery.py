"""
HttpDiscovery — centralised discovery adapter (optional extension).

Connects to any REST-based agent registry that implements the Borgkit
centralised discovery API. This is NOT the default; LocalDiscovery and
GossipDiscovery are preferred. Use this as an escape hatch when:
  - bootstrapping a new network
  - interoperating with a managed registry
  - operating in an enterprise / firewalled environment

The server must expose:
  POST   /agents           → register
  DELETE /agents/{id}      → unregister
  GET    /agents?cap=X     → query by capability
  GET    /agents           → list all
  PUT    /agents/{id}/hb   → heartbeat

Install dep: pip install aiohttp
"""

from __future__ import annotations

import asyncio
import os
import dataclasses
import json
from typing import Optional

try:
    import aiohttp
    _AIOHTTP_OK = True
except ImportError:
    _AIOHTTP_OK = False

from interfaces.iagent_discovery import IAgentDiscovery, DiscoveryEntry, NetworkInfo, HealthStatus


class HttpDiscovery(IAgentDiscovery):
    """Centralised REST registry adapter — optional extension."""

    def __init__(
        self,
        base_url: str,
        api_key: Optional[str] = None,
        timeout_ms: int = 5_000,
        heartbeat_interval_ms: int = 30_000,
    ):
        if not _AIOHTTP_OK:
            raise RuntimeError("aiohttp not installed — pip install aiohttp")
        self._base     = base_url.rstrip('/')
        self._headers  = {'Content-Type': 'application/json'}
        if api_key:
            self._headers['X-Api-Key'] = api_key
        self._timeout  = aiohttp.ClientTimeout(total=timeout_ms / 1000)
        self._hb_ms    = heartbeat_interval_ms
        self._hb_tasks: dict[str, asyncio.Task] = {}

    # ── IAgentDiscovery impl ──────────────────────────────────────────────────

    async def register(self, entry: DiscoveryEntry) -> None:
        await self._request('POST', '/agents', body=_entry_to_dict(entry))
        if self._hb_ms > 0:
            self._hb_tasks[entry.agent_id] = asyncio.create_task(
                self._heartbeat_loop(entry.agent_id)
            )
        print(f'[HttpDiscovery] Registered: {entry.agent_id} → {self._base}')

    async def unregister(self, agent_id: str) -> None:
        task = self._hb_tasks.pop(agent_id, None)
        if task:
            task.cancel()
        await self._request('DELETE', f'/agents/{_enc(agent_id)}')

    async def query(self, capability: str) -> list[DiscoveryEntry]:
        data = await self._request('GET', f'/agents?cap={_enc(capability)}')
        return [_dict_to_entry(d) for d in (data or [])]

    async def list_all(self) -> list[DiscoveryEntry]:
        data = await self._request('GET', '/agents')
        return [_dict_to_entry(d) for d in (data or [])]

    async def heartbeat(self, agent_id: str) -> None:
        await self._request('PUT', f'/agents/{_enc(agent_id)}/hb')

    # ── internals ─────────────────────────────────────────────────────────────

    async def _heartbeat_loop(self, agent_id: str) -> None:
        while True:
            await asyncio.sleep(self._hb_ms / 1000)
            try:
                await self.heartbeat(agent_id)
            except Exception as e:
                print(f'[HttpDiscovery] heartbeat failed for {agent_id}: {e}')

    async def _request(self, method: str, path: str, body=None):
        async with aiohttp.ClientSession(headers=self._headers, timeout=self._timeout) as session:
            url = f'{self._base}{path}'
            kwargs = {'json': body} if body else {}
            async with session.request(method, url, **kwargs) as resp:
                resp.raise_for_status()
                if resp.content_type == 'application/json' and resp.status != 204:
                    return await resp.json()
                return None


# ── DiscoveryFactory ──────────────────────────────────────────────────────────

class DiscoveryFactory:
    """
    Selects the appropriate discovery backend.

    Priority:
      1. explicit type argument
      2. BORGKIT_P2P=true env var          → Libp2pDiscovery
      3. BORGKIT_DISCOVERY_URL env var     → HttpDiscovery
      4. default                           → LocalDiscovery
    """

    @staticmethod
    async def create(
        discovery_type: Optional[str] = None,
        http_base_url: Optional[str] = None,
        api_key: Optional[str] = None,
        libp2p_config=None,
        onchain_config: Optional[dict] = None,
    ) -> IAgentDiscovery:
        from discovery.local_discovery import LocalDiscovery

        t = discovery_type or (
            'libp2p' if os.environ.get('BORGKIT_P2P') == 'true' else
            'http'   if os.environ.get('BORGKIT_DISCOVERY_URL') else
            'local'
        )

        if t == 'http':
            url = http_base_url or os.environ.get('BORGKIT_DISCOVERY_URL')
            if not url:
                raise ValueError('HttpDiscovery requires a base URL')
            return HttpDiscovery(
                base_url=url,
                api_key=api_key or os.environ.get('BORGKIT_DISCOVERY_KEY'),
            )

        if t == 'libp2p':
            from discovery.libp2p_discovery import Libp2pDiscovery, Libp2pDiscoveryConfig
            cfg = libp2p_config or Libp2pDiscoveryConfig()
            return await Libp2pDiscovery.start(cfg)

        if t == 'onchain':
            from discovery.onchain_discovery import OnChainDiscovery, OnChainDiscoveryConfig
            cfg = OnChainDiscoveryConfig(
                rpc_url          = onchain_config.get('rpcUrl', '') if onchain_config else os.environ.get('BORGKIT_RPC_URL', ''),
                contract_address = onchain_config.get('contractAddress', '') if onchain_config else os.environ.get('BORGKIT_CONTRACT_ADDRESS', ''),
                private_key      = onchain_config.get('privateKey', '') if onchain_config else os.environ.get('BORGKIT_PRIVATE_KEY', ''),
                chain_id         = onchain_config.get('chainId', 8453) if onchain_config else int(os.environ.get('BORGKIT_CHAIN_ID', '8453')),
            )
            return OnChainDiscovery(cfg)

        return LocalDiscovery.get_instance()


# ── helpers ───────────────────────────────────────────────────────────────────

def _enc(s: str) -> str:
    from urllib.parse import quote
    return quote(s, safe='')

def _entry_to_dict(e: DiscoveryEntry) -> dict:
    return {
        'agentId':      e.agent_id,
        'name':         e.name,
        'owner':        e.owner,
        'capabilities': e.capabilities,
        'network': {
            'protocol': e.network.protocol,
            'host':     e.network.host,
            'port':     e.network.port,
            'tls':      e.network.tls,
        },
        'health': {
            'status':        e.health.status,
            'lastHeartbeat': e.health.last_heartbeat,
            'uptimeSeconds': e.health.uptime_seconds,
        },
        'registeredAt': e.registered_at,
        'metadataUri':  e.metadata_uri,
    }

def _dict_to_entry(d: dict) -> DiscoveryEntry:
    net = d.get('network', {})
    hlt = d.get('health', {})
    return DiscoveryEntry(
        agent_id=d['agentId'],
        name=d['name'],
        owner=d.get('owner', ''),
        capabilities=d.get('capabilities', []),
        network=NetworkInfo(
            protocol=net.get('protocol', 'http'),
            host=net.get('host', ''),
            port=net.get('port', 0),
            tls=net.get('tls', False),
        ),
        health=HealthStatus(
            status=hlt.get('status', 'unknown'),
            last_heartbeat=hlt.get('lastHeartbeat', ''),
            uptime_seconds=hlt.get('uptimeSeconds', 0),
        ),
        registered_at=d.get('registeredAt', ''),
        metadata_uri=d.get('metadataUri'),
    )
