"""
Standard request envelope for all agent-to-agent calls.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Optional, TYPE_CHECKING

if TYPE_CHECKING:
    from addons.x402.types import X402Payment


@dataclass
class PaymentInfo:
    type: str           # "oneshot" | "stream" | "subscription"
    token: str          # e.g. "USDC", "ETH"
    amount: str         # human-readable, e.g. "0.001"
    tx_hash: Optional[str] = None


@dataclass
class AgentRequest:
    request_id: str
    from_id: str                              # caller agent ID or wallet address
    capability: str
    payload: Dict[str, Any] = field(default_factory=dict)
    signature: Optional[str] = None          # EIP-712 over this envelope
    timestamp: Optional[int] = None          # Unix ms — rejects stale requests
    session_key: Optional[str] = None        # Delegated execution session key
    payment: Optional[PaymentInfo] = None
    # ── x402 micropayment proof (set by X402Client on retry) ──────────────
    x402: Optional[Any] = None               # X402Payment — see addons/x402
    # ── streaming flag — set to True to use POST /invoke/stream SSE ───────
    stream: bool = False

    def to_dict(self) -> dict:
        d = {
            "requestId":  self.request_id,
            "from":       self.from_id,
            "capability": self.capability,
            "payload":    self.payload,
            "signature":  self.signature,
            "timestamp":  self.timestamp,
        }
        if self.stream:
            d["stream"] = True
        return d
