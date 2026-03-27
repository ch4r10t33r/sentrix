"""
x402 Server Mixin
─────────────────────────────────────────────────────────────────────────────
Adds payment enforcement to any Borgkit IAgent.

Usage
-----
    from addons.x402 import X402ServerMixin, CapabilityPricing
    from interfaces  import IAgent, AgentRequest, AgentResponse

    class MyPaidAgent(X402ServerMixin, IAgent):
        agent_id = "borgkit://agent/my-paid-agent"
        owner    = "0xMyWalletAddress"

        # ── pricing table ──────────────────────────────────────────────────
        x402_pricing = {
            "generate_image": CapabilityPricing.usdc_base(
                amount_usd_cents=50,
                pay_to="0xMyWalletAddress",
                description="Image generation — $0.50 per request",
            ),
            "translate_text": CapabilityPricing(
                network="base",
                asset="0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
                amount="100000",   # 0.10 USDC
                pay_to="0xMyWalletAddress",
            ),
        }

        def get_capabilities(self):
            return ["generate_image", "translate_text"]

        # ── implement your actual logic here ───────────────────────────────
        async def _handle_paid_request(self, req: AgentRequest) -> AgentResponse:
            if req.capability == "generate_image":
                return AgentResponse.success(req.request_id, {"url": "https://..."})
            if req.capability == "translate_text":
                return AgentResponse.success(req.request_id, {"text": "..."})
            return AgentResponse.error(req.request_id, "Unknown capability")

Verification
------------
By default X402ServerMixin runs in *dev mode* — it accepts any payment proof
without cryptographic verification.  For production, either:

  1. Override _verify_x402_payment() with your own on-chain check, OR
  2. Use X402Facilitator (recommended) for off-chain verification:

        async def _verify_x402_payment(self, payment, pricing, req):
            from addons.x402 import X402Facilitator
            fac = X402Facilitator(base_url="https://x402.org/facilitator")
            return await fac.verify(payment, pricing.to_requirements(memo=req.request_id))
"""

from __future__ import annotations

import warnings
from abc import abstractmethod
from typing import Dict, Optional

from interfaces import AgentRequest, AgentResponse
from .types import CapabilityPricing, X402Payment, X402Receipt, X402PaymentRequirements


class X402ServerMixin:
    """
    Mixin that adds x402 payment enforcement to any IAgent subclass.

    Mix-in order: X402ServerMixin must come BEFORE IAgent in the MRO so that
    handle_request() is intercepted before the agent's own implementation.

        class MyAgent(X402ServerMixin, IAgent): ...
    """

    #: Map of capability name → pricing config.
    #: Capabilities not listed here are served free of charge.
    x402_pricing: Dict[str, CapabilityPricing] = {}

    #: Set to True to enable strict verification (rejects unsigned/invalid proofs).
    #: In dev mode (False) all payment proofs are accepted without verification.
    x402_strict: bool = False

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        """
        Intercepts handle_request() to enforce payment before delegating.
        """
        pricing = self.x402_pricing.get(req.capability)

        if pricing is not None:
            # No payment proof included → return payment requirements
            payment: Optional[X402Payment] = getattr(req, "x402", None)
            if payment is None:
                return self._payment_required_response(req, pricing)

            # Verify the proof
            receipt = await self._verify_x402_payment(payment, pricing, req)
            if not receipt.success:
                return AgentResponse.error(
                    req.request_id,
                    f"x402 payment verification failed: {receipt.error_reason or 'unknown reason'}",
                )

        # Payment satisfied (or capability is free) — dispatch to real handler
        return await self._handle_paid_request(req)

    def _payment_required_response(
        self,
        req: AgentRequest,
        pricing: CapabilityPricing,
    ) -> AgentResponse:
        """Build an AgentResponse with status='payment_required'."""
        requirements = pricing.to_requirements(memo=req.request_id)
        resp = AgentResponse(
            request_id=req.request_id,
            status="payment_required",
            result=None,
            error_message=(
                f"Capability '{req.capability}' requires payment. "
                f"See payment_requirements for details."
            ),
        )
        # Attach requirements — AgentResponse has this field if using the
        # updated interface; otherwise set it dynamically.
        resp.payment_requirements = [requirements.to_dict()]   # type: ignore[attr-defined]
        return resp

    async def _verify_x402_payment(
        self,
        payment: X402Payment,
        pricing: CapabilityPricing,
        req: AgentRequest,
    ) -> X402Receipt:
        """
        Verify an x402 payment proof.

        Default implementation: DEV MODE — accepts all proofs with a warning.
        Override for production, or use X402Facilitator.

        Example override:
            async def _verify_x402_payment(self, payment, pricing, req):
                from addons.x402 import X402Facilitator
                return await X402Facilitator().verify(
                    payment, pricing.to_requirements(memo=req.request_id)
                )
        """
        if self.x402_strict:
            return X402Receipt(
                success=False,
                error_reason=(
                    "x402_strict=True but _verify_x402_payment() not overridden. "
                    "Override this method or use X402Facilitator."
                ),
            )
        warnings.warn(
            f"[x402] DEV MODE: payment proof for '{req.capability}' accepted without "
            "verification. Override _verify_x402_payment() or use X402Facilitator "
            "before going to production.",
            stacklevel=4,
        )
        return X402Receipt(success=True)

    @abstractmethod
    async def _handle_paid_request(self, req: AgentRequest) -> AgentResponse:
        """
        Implement your capability logic here.
        This is only called after payment has been verified (or the capability is free).
        """
        ...
