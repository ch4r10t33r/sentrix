"""
Libp2PGossipProtocol — IGossipProtocol backed by the Rust FFI GossipSub.
"""
from __future__ import annotations

import json
from typing import Awaitable, Callable, List

from interfaces.iagent_mesh import GossipMessage, IGossipProtocol
from .ffi import BorgkitLibp2P

GossipHandler = Callable[[GossipMessage], Awaitable[None]]


class Libp2PGossipProtocol(IGossipProtocol):
    """
    Publishes gossip messages via the Rust borgkit-libp2p GossipSub.
    Incoming messages arrive via the request handler registered on the FFI node
    (capability == "__gossip_pubsub") and are forwarded to registered handlers.
    """

    def __init__(self, ffi: BorgkitLibp2P) -> None:
        self._ffi      = ffi
        self._handlers: List[GossipHandler] = []
        self._peers:    List[str] = []

    async def broadcast(self, message: GossipMessage) -> None:
        self._ffi.gossip_publish(json.dumps(message.to_dict()))

    async def receive(self, message: GossipMessage) -> None:
        if not message.should_forward:
            return
        for h in self._handlers:
            try:
                await h(message)
            except Exception:
                pass

    def subscribe(self, handler: GossipHandler) -> None:
        self._handlers.append(handler)

    def peers(self) -> List[str]:
        return list(self._peers)

    async def add_peer(self, agent_id: str, endpoint: str) -> None:
        try:
            self._ffi.dial(endpoint)
            self._peers.append(agent_id)
        except ConnectionError:
            pass

    async def remove_peer(self, agent_id: str) -> None:
        self._peers = [p for p in self._peers if p != agent_id]
