"""
Borgkit x402 Payment Add-on
────────────────────────────────────────────────────────────────────────────
Implements the x402 micropayment protocol (https://x402.org) for
agent-to-agent capability pricing.

Server side — charge callers for capabilities:
    from addons.x402 import X402ServerMixin, CapabilityPricing

    class MyAgent(X402ServerMixin, IAgent):
        x402_pricing = {
            'premium_search': CapabilityPricing(
                network='base',
                asset='0x833589fcd6edb6e08f4c7c32d4f71b54bda02913',  # USDC on Base
                amount='1000000',   # 1 USDC (6 decimals)
                pay_to='0xYourWalletAddress',
            )
        }
        async def _handle_paid_request(self, req): ...

Client side — pay automatically when calling paid capabilities:
    from addons.x402 import X402Client

    client  = X402Client(wallet=my_wallet)
    resp    = await client.call(agent, req)
    # Pays automatically if the agent returns payment_required

Facilitator (optional — for server-side payment verification):
    from addons.x402 import X402Facilitator

    fac     = X402Facilitator()
    receipt = await fac.verify(payment, requirements)
"""

from .types import (
    X402PaymentRequirements,
    X402Payment,
    X402Receipt,
    CapabilityPricing,
)
from .server import X402ServerMixin
from .client import X402Client
from .facilitator import X402Facilitator

__all__ = [
    "X402PaymentRequirements",
    "X402Payment",
    "X402Receipt",
    "CapabilityPricing",
    "X402ServerMixin",
    "X402Client",
    "X402Facilitator",
]
