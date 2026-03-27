"""
x402 Client
─────────────────────────────────────────────────────────────────────────────
Adds automatic payment handling to agent-to-agent calls.

When an agent returns status='payment_required', X402Client reads the payment
requirements, signs the authorisation, and retries the request — all
transparently to the caller.

Usage
-----
Basic (inspect payment_required manually):
    from addons.x402 import X402Client

    client = X402Client()   # no wallet — returns payment_required as-is
    resp   = await client.call(agent, req)
    if resp.status == 'payment_required':
        print("Payment needed:", resp.payment_requirements)

With a wallet (auto-pays):
    from addons.x402        import X402Client
    from addons.x402.wallet import CoinbaseWalletProvider   # optional helper

    wallet = CoinbaseWalletProvider(private_key_hex=os.environ["WALLET_KEY"])
    client = X402Client(wallet=wallet, auto_pay=True)
    resp   = await client.call(agent, req)   # pays automatically

Custom wallet (implement WalletProvider ABC):
    class MyWallet(WalletProvider):
        async def sign_payment(self, req, requirements): ...

    client = X402Client(wallet=MyWallet(), auto_pay=True)
"""

from __future__ import annotations

import copy
import warnings
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Optional

from interfaces import AgentRequest, AgentResponse
from .types import X402Payment, X402PaymentRequirements, X402Receipt


# ── wallet provider interface ─────────────────────────────────────────────────

class WalletProvider(ABC):
    """
    Abstract wallet interface for the x402 client.

    Implement this to connect any wallet (Coinbase, MetaMask, hardware, etc.)
    to the Borgkit x402 client.
    """

    @abstractmethod
    async def sign_payment(
        self,
        requirements: X402PaymentRequirements,
        original_request: AgentRequest,
    ) -> X402Payment:
        """
        Sign a payment authorisation.

        Args:
            requirements:     What the server expects (network, asset, amount, payTo).
            original_request: The original AgentRequest (for memo / correlation).

        Returns:
            X402Payment with a signed EIP-3009 (ERC-20 transferWithAuthorization)
            or equivalent payload.
        """
        ...

    @abstractmethod
    def address(self) -> str:
        """Return the wallet's Ethereum address."""
        ...


# ── dev/mock wallet ───────────────────────────────────────────────────────────

class MockWalletProvider(WalletProvider):
    """
    Mock wallet for development and testing.

    Returns unsigned dummy payment proofs — accepted by X402ServerMixin in dev
    mode.  NOT suitable for production.
    """

    def __init__(self, address: str = "0xDevWallet0000000000000000000000000000000"):
        self._address = address

    def address(self) -> str:
        return self._address

    async def sign_payment(
        self,
        requirements: X402PaymentRequirements,
        original_request: AgentRequest,
    ) -> X402Payment:
        warnings.warn(
            "[x402] MockWalletProvider: returning unsigned dummy payment. "
            "Use a real WalletProvider in production.",
            stacklevel=3,
        )
        return X402Payment(
            x402_version=1,
            scheme=requirements.scheme,
            network=requirements.network,
            payload="mock-unsigned-payload",
            signature="mock-signature",
        )


# ── x402 client ───────────────────────────────────────────────────────────────

@dataclass
class X402Client:
    """
    Wraps calls to any IAgent and handles x402 payment negotiation automatically.

    Args:
        wallet:    A WalletProvider that signs payment authorisations.
                   If None, payment_required responses are returned to the caller.
        auto_pay:  If True, pays automatically without confirmation.
                   If False (default), calls on_payment_required() which you can
                   override to prompt the user.
        max_retries: Maximum number of retry attempts after payment (default: 1).
    """

    wallet:      Optional[WalletProvider] = None
    auto_pay:    bool = False
    max_retries: int  = 1

    async def call(self, agent, req: AgentRequest) -> AgentResponse:
        """
        Call an agent, handling x402 payment_required responses automatically.

        If the agent returns payment_required and a wallet is configured,
        signs the payment and retries up to max_retries times.
        """
        resp = await agent.handle_request(req)

        if resp.status != "payment_required":
            return resp

        if self.wallet is None:
            # No wallet — return the payment_required response for the caller to handle
            return resp

        # Parse requirements from response
        raw_reqs = getattr(resp, "payment_requirements", None) or []
        if not raw_reqs:
            return AgentResponse.error(
                req.request_id,
                "payment_required response missing payment_requirements",
            )

        requirements = X402PaymentRequirements.from_dict(raw_reqs[0])

        # Optionally confirm with the user / caller
        if not self.auto_pay:
            confirmed = await self.on_payment_required(requirements, req, resp)
            if not confirmed:
                return resp  # return original payment_required

        # Sign and build payment
        payment = await self.wallet.sign_payment(requirements, req)

        # Retry with payment proof
        for attempt in range(self.max_retries):
            paid_req = self._attach_payment(req, payment)
            retry_resp = await agent.handle_request(paid_req)

            if retry_resp.status != "payment_required":
                return retry_resp

            # Still asking for payment — something went wrong
            if attempt < self.max_retries - 1:
                warnings.warn(
                    f"[x402] Payment retry {attempt + 1}/{self.max_retries} — "
                    "server still returned payment_required"
                )

        return AgentResponse.error(
            req.request_id,
            f"x402 payment failed after {self.max_retries} attempt(s)",
        )

    async def on_payment_required(
        self,
        requirements: X402PaymentRequirements,
        original_req: AgentRequest,
        payment_resp: AgentResponse,
    ) -> bool:
        """
        Called when auto_pay=False and payment is required.

        Override this method to prompt the user for confirmation.
        Default: returns True (pays automatically despite auto_pay=False).

        Return True to proceed with payment, False to abort.
        """
        # Default: print and proceed
        print(
            f"[x402] Payment required for '{original_req.capability}':\n"
            f"       Network : {requirements.network}\n"
            f"       Asset   : {requirements.asset}\n"
            f"       Amount  : {requirements.max_amount_required}\n"
            f"       Pay to  : {requirements.pay_to}\n"
            f"       Desc    : {requirements.description or '(none)'}"
        )
        return True  # override to add confirmation prompt

    @staticmethod
    def _attach_payment(req: AgentRequest, payment: X402Payment) -> AgentRequest:
        """Return a copy of the request with the x402 payment attached."""
        req_copy = copy.copy(req)
        req_copy.x402 = payment   # type: ignore[attr-defined]
        return req_copy
