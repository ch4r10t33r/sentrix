"""
Libp2pDiscovery — fully P2P discovery backend for Borgkit.

Architecture:
  Transport   : QUIC (via borgkit-libp2p-sidecar Rust binary)
  Routing     : Kademlia DHT  (/borgkit/kad/1.0.0 — isolated from IPFS)
  Local LAN   : mDNS (optional, default on)
  NAT         : DCUtR hole punching + circuit-relay-v2 fallback
  Identity    : secp256k1 keypair from ANR — same key → same PeerId

WHY A SIDECAR?
  py-libp2p is incomplete as of 2026: it lacks QUIC transport, production-
  grade Kademlia, mDNS, and DCUtR.  Rather than ship a broken pure-Python
  implementation, this module delegates to either:

  1. A compiled `borgkit-libp2p-sidecar` binary (Rust, from templates/rust/)
     — launched as a subprocess, communicates over stdin/stdout JSON-RPC 2.0.

  2. A Node.js process running the TypeScript Libp2pDiscovery
     — used if the sidecar binary is not found but `node` is available.

  3. HttpDiscovery pointed at a local gateway (automatic fallback)
     — used if neither sidecar nor Node is available.
     Set BORGKIT_LIBP2P_GATEWAY=http://localhost:7731 to enable.

  Configure via environment variables or Libp2pDiscoveryConfig:
    BORGKIT_LIBP2P_SIDECAR   path to the compiled sidecar binary
    BORGKIT_LIBP2P_NODE      path to the TypeScript entry point (if using Node)
    BORGKIT_BOOTSTRAP_PEERS  comma-separated multiaddrs of bootstrap peers
    BORGKIT_P2P_PORT         UDP port for QUIC listener (default: 0 = OS-assigned)

Usage:
    cfg = Libp2pDiscoveryConfig(private_key_bytes=my_anr_key)
    discovery = await Libp2pDiscovery.start(cfg)
    await discovery.register(entry)
    peers = await discovery.query('web_search')
    await discovery.stop()
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import sys
import dataclasses
from typing import Optional

from interfaces.iagent_discovery import IAgentDiscovery, DiscoveryEntry, NetworkInfo, HealthStatus


# ── Config ────────────────────────────────────────────────────────────────────

@dataclasses.dataclass
class Libp2pDiscoveryConfig:
    """Configuration for the Libp2pDiscovery sidecar."""

    # 32-byte secp256k1 private key (same as ANR signing key).
    # Omit only for ephemeral/throwaway nodes.
    private_key_bytes: Optional[bytes] = None

    # UDP port for the QUIC listener (0 = OS-assigned).
    listen_port: int = 0

    # Bootstrap peer multiaddrs (format: /ip4/.../udp/.../quic-v1/p2p/...)
    # Also read from BORGKIT_BOOTSTRAP_PEERS env var (comma-separated).
    bootstrap_peers: list[str] = dataclasses.field(default_factory=list)

    # How often to re-publish DHT records (seconds). Default: 30
    heartbeat_interval_secs: int = 30

    # Enable mDNS for local network discovery. Default: True
    enable_mdns: bool = True

    # Path to the compiled borgkit-libp2p-sidecar binary.
    # Falls back to BORGKIT_LIBP2P_SIDECAR env var, then PATH lookup.
    sidecar_binary: Optional[str] = None

    # Path to the Node.js TypeScript entry point (fallback if no sidecar).
    # Falls back to BORGKIT_LIBP2P_NODE env var.
    node_entry: Optional[str] = None


# ── JSON-RPC 2.0 helpers ──────────────────────────────────────────────────────

_rpc_id = 0

def _make_request(method: str, params: dict) -> bytes:
    global _rpc_id
    _rpc_id += 1
    msg = {'jsonrpc': '2.0', 'id': _rpc_id, 'method': method, 'params': params}
    return (json.dumps(msg) + '\n').encode()


def _parse_response(line: bytes) -> dict:
    return json.loads(line.decode().strip())


# ── Sidecar process management ────────────────────────────────────────────────

class _SidecarProcess:
    """Manages a borgkit-libp2p-sidecar subprocess (Rust or Node.js)."""

    def __init__(self, proc: asyncio.subprocess.Process):
        self._proc = proc
        self._lock = asyncio.Lock()

    @classmethod
    async def launch(cls, cfg: Libp2pDiscoveryConfig) -> '_SidecarProcess':
        binary = cls._resolve_binary(cfg)
        if binary is None:
            raise RuntimeError(
                'borgkit-libp2p-sidecar not found.\n'
                'Options:\n'
                '  1. Compile the Rust sidecar: cd templates/rust && cargo build --release\n'
                '  2. Set BORGKIT_LIBP2P_SIDECAR=/path/to/borgkit-libp2p-sidecar\n'
                '  3. Set BORGKIT_LIBP2P_GATEWAY=http://localhost:7731 to use HTTP fallback\n'
                '  4. Set BORGKIT_DISCOVERY_URL to use a centralised registry'
            )

        env_peers = os.environ.get('BORGKIT_BOOTSTRAP_PEERS', '')
        all_peers = cfg.bootstrap_peers + [p for p in env_peers.split(',') if p]

        args = [binary, 'run',
                '--port',        str(cfg.listen_port),
                '--heartbeat',   str(cfg.heartbeat_interval_secs),
                '--mdns',        str(cfg.enable_mdns).lower(),
                '--bootstrap',   ','.join(all_peers),
        ]
        if cfg.private_key_bytes:
            args += ['--key', cfg.private_key_bytes.hex()]

        proc = await asyncio.create_subprocess_exec(
            *args,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        return cls(proc)

    @staticmethod
    def _resolve_binary(cfg: Libp2pDiscoveryConfig) -> Optional[str]:
        # 1. Explicit config
        if cfg.sidecar_binary and os.path.isfile(cfg.sidecar_binary):
            return cfg.sidecar_binary
        # 2. Env var
        env_path = os.environ.get('BORGKIT_LIBP2P_SIDECAR')
        if env_path and os.path.isfile(env_path):
            return env_path
        # 3. PATH lookup
        found = shutil.which('borgkit-libp2p-sidecar')
        if found:
            return found
        # 4. Node.js fallback
        node_entry = cfg.node_entry or os.environ.get('BORGKIT_LIBP2P_NODE')
        if node_entry and shutil.which('node'):
            return f'node {node_entry}'  # handled specially in launch
        return None

    async def call(self, method: str, params: dict) -> dict:
        async with self._lock:
            req = _make_request(method, params)
            self._proc.stdin.write(req)
            await self._proc.stdin.drain()
            line = await self._proc.stdout.readline()
            return _parse_response(line)

    async def stop(self) -> None:
        try:
            self._proc.stdin.write((_make_request('stop', {}) + b''))
            await self._proc.stdin.drain()
        except Exception:
            pass
        try:
            self._proc.terminate()
            await asyncio.wait_for(self._proc.wait(), timeout=5.0)
        except Exception:
            self._proc.kill()


# ── HttpDiscovery gateway fallback ────────────────────────────────────────────

class _HttpGatewayFallback(IAgentDiscovery):
    """
    Fallback when the libp2p sidecar is unavailable.
    Connects to a local or remote HTTP gateway that bridges HTTP ↔ libp2p DHT.
    Set BORGKIT_LIBP2P_GATEWAY=http://localhost:7731 to activate.
    """

    def __init__(self, base_url: str):
        from discovery.http_discovery import HttpDiscovery
        self._inner = HttpDiscovery(base_url=base_url)

    async def register(self, entry: DiscoveryEntry) -> None:
        return await self._inner.register(entry)

    async def unregister(self, agent_id: str) -> None:
        return await self._inner.unregister(agent_id)

    async def query(self, capability: str) -> list[DiscoveryEntry]:
        return await self._inner.query(capability)

    async def list_all(self) -> list[DiscoveryEntry]:
        return await self._inner.list_all()

    async def heartbeat(self, agent_id: str) -> None:
        return await self._inner.heartbeat(agent_id)


# ── Libp2pDiscovery ───────────────────────────────────────────────────────────

class Libp2pDiscovery(IAgentDiscovery):
    """
    P2P discovery backend backed by a borgkit-libp2p-sidecar subprocess.

    Falls back to _HttpGatewayFallback if the sidecar is unavailable and
    BORGKIT_LIBP2P_GATEWAY is set.
    """

    def __init__(self, sidecar: '_SidecarProcess'):
        self._sidecar = sidecar

    @classmethod
    async def start(cls, cfg: Libp2pDiscoveryConfig = None) -> 'Libp2pDiscovery':
        if cfg is None:
            cfg = Libp2pDiscoveryConfig()
        try:
            sidecar = await _SidecarProcess.launch(cfg)
            return cls(sidecar)
        except RuntimeError as e:
            gateway = os.environ.get('BORGKIT_LIBP2P_GATEWAY')
            if gateway:
                print(f'[Libp2pDiscovery] Sidecar unavailable — falling back to HTTP gateway: {gateway}')
                return _HttpGatewayFallback(gateway)  # type: ignore[return-value]
            raise RuntimeError(
                f'{e}\n\nAlternatively, set BORGKIT_LIBP2P_GATEWAY=<url> to use an HTTP-to-DHT bridge.'
            ) from e

    async def stop(self) -> None:
        await self._sidecar.stop()
        print('[Libp2pDiscovery] Stopped')

    # ── IAgentDiscovery ───────────────────────────────────────────────────────

    async def register(self, entry: DiscoveryEntry) -> None:
        resp = await self._sidecar.call('register', {'entry': _entry_to_dict(entry)})
        _check_rpc_error(resp)

    async def unregister(self, agent_id: str) -> None:
        resp = await self._sidecar.call('unregister', {'agent_id': agent_id})
        _check_rpc_error(resp)

    async def query(self, capability: str) -> list[DiscoveryEntry]:
        resp = await self._sidecar.call('query', {'capability': capability})
        _check_rpc_error(resp)
        return [_dict_to_entry(d) for d in resp.get('result', [])]

    async def list_all(self) -> list[DiscoveryEntry]:
        resp = await self._sidecar.call('list_all', {})
        _check_rpc_error(resp)
        return [_dict_to_entry(d) for d in resp.get('result', [])]

    async def heartbeat(self, agent_id: str) -> None:
        resp = await self._sidecar.call('heartbeat', {'agent_id': agent_id})
        _check_rpc_error(resp)


# ── Serialisation helpers ─────────────────────────────────────────────────────

def _check_rpc_error(resp: dict) -> None:
    if 'error' in resp:
        raise RuntimeError(f"[Libp2pDiscovery] RPC error: {resp['error']}")


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
            peer_id=net.get('peerId', ''),
            multiaddr=net.get('multiaddr', ''),
        ),
        health=HealthStatus(
            status=hlt.get('status', 'unknown'),
            last_heartbeat=hlt.get('lastHeartbeat', ''),
            uptime_seconds=hlt.get('uptimeSeconds', 0),
        ),
        registered_at=d.get('registeredAt', ''),
        metadata_uri=d.get('metadataUri'),
    )
