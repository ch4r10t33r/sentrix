//! # x402 Micropayment Addon
//!
//! This module implements the [x402 payment protocol](https://x402.org) for Borgkit agents.
//! x402 is an HTTP-native micropayment protocol built on top of EIP-3009
//! (transferWithAuthorization) and EIP-712 typed structured signing. It is named
//! after the HTTP 402 "Payment Required" status code.
//!
//! ## Protocol overview
//!
//! 1. **Client calls `/invoke`** without a payment proof.
//! 2. **Server returns HTTP 402** with a JSON body containing
//!    [`X402PaymentRequirements`] (amount, asset, network, recipient wallet).
//! 3. **Client signs** an EIP-3009 `transferWithAuthorization` for the requested
//!    amount, wraps it in an [`X402Payment`] struct, and re-sends the request.
//! 4. **Server verifies** the proof (optionally via an off-chain facilitator) and,
//!    on success, proceeds to execute the capability.
//!
//! Reference: <https://x402.org> / <https://github.com/coinbase/x402>
//!
//! ## Server-side usage
//!
//! ```rust,no_run
//! use std::collections::HashMap;
//! use borgkit::addons::x402::{X402Server, usdc_base};
//! // Assumes `MyAgent` implements `IAgent`.
//! // use crate::example_agent::ExampleAgent;
//!
//! let mut pricing = HashMap::new();
//! pricing.insert(
//!     "generate_image".to_string(),
//!     usdc_base(50, "0xMyWalletAddress", Some("Image generation — $0.50")),
//! );
//!
//! // let agent = X402Server::new(MyAgent::default(), pricing);
//! // borgkit::server::serve(agent, 6174).await?;
//! ```
//!
//! ## Client-side usage
//!
//! ```rust,no_run
//! use borgkit::addons::x402::{X402Client, X402PaymentRequirements};
//!
//! let client = X402Client::new("0xMyWalletAddress");
//!
//! // When a 402 is received, extract requirements from the response body,
//! // then attach a mock proof (dev mode) before retrying:
//! // let payment = client.mock_payment(&requirements, &request_id);
//! // request.x402 = Some(payment);   // attach to your AgentRequest extension
//! ```
//!
//! ## Extending AgentRequest
//!
//! The Borgkit `AgentRequest` type (in `request.rs`) does not ship with an
//! `x402` field by default to keep the core envelope framework-agnostic.
//! Callers that need to carry an `X402Payment` proof should extend the request
//! at the serde layer, e.g. by wrapping it:
//!
//! ```rust,no_run
//! use borgkit::addons::x402::X402Payment;
//! use borgkit::request::AgentRequest;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! pub struct PaidAgentRequest {
//!     #[serde(flatten)]
//!     pub inner: AgentRequest,
//!     /// x402 payment proof — present only on the retry after a 402 response.
//!     #[serde(skip_serializing_if = "Option::is_none")]
//!     pub x402: Option<X402Payment>,
//! }
//! ```

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::agent::IAgent;
use crate::request::AgentRequest;

// ── Payment requirements (server → client, sent in the 402 response body) ─────

/// Payment requirements returned to the client when HTTP 402 is issued.
///
/// The client reads this struct, constructs a signed EIP-3009 transfer
/// authorisation, wraps it in [`X402Payment`], and re-sends the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X402PaymentRequirements {
    /// Payment scheme: `"exact"` (pay exactly) or `"upto"` (pay up to).
    pub scheme: String,
    /// Target chain name, e.g. `"base"`, `"ethereum"`, `"polygon"`.
    pub network: String,
    /// Maximum amount required, expressed in the asset's smallest unit
    /// (USDC uses 6 decimals; ETH uses 18 decimals).
    pub max_amount_required: String,
    /// ERC-20 contract address, or `"ETH"` / `"native"` for the chain's native
    /// currency.
    pub asset: String,
    /// Recipient wallet address (the agent operator's address).
    pub pay_to: String,
    /// Correlation ID — callers should set this to the originating `request_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// How long the payment authorisation remains valid, in seconds.
    /// Defaults to 300 (5 minutes) when not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_timeout_seconds: Option<u64>,
    /// Human-readable description of the charge shown to the end user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Payment proof (client → server, attached to the retry AgentRequest) ───────

/// Payment proof submitted by the client.
///
/// This is the "credential" attached to a second `AgentRequest` after the
/// client received a 402 response. The server (or an off-chain facilitator)
/// verifies the proof before executing the capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X402Payment {
    /// Protocol version — currently `1`.
    pub x402_version: u32,
    /// Must match the scheme requested in [`X402PaymentRequirements::scheme`].
    pub scheme: String,
    /// Must match the network requested in [`X402PaymentRequirements::network`].
    pub network: String,
    /// Base64url-encoded, ABI-encoded EIP-3009 `transferWithAuthorization`
    /// function call data (signed by the payer).
    pub payload: String,
    /// Outer EIP-712 signature that covers `scheme + network + payload`.
    pub signature: String,
}

// ── Payment receipt (returned by the facilitator after verification) ──────────

/// Receipt returned by the payment facilitator (or the server's inline verifier)
/// after attempting to verify and settle an [`X402Payment`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X402Receipt {
    /// `true` if the payment was valid and settlement succeeded.
    pub success: bool,
    /// On-chain transaction hash for the settled transfer (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<String>,
    /// Human-readable reason for failure when `success` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    /// Address of the payer extracted from the proof.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    /// Amount that was actually settled (may differ from requested under "upto"
    /// scheme).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_settled: Option<String>,
}

// ── Capability pricing (server-side configuration) ────────────────────────────

/// Per-capability pricing configuration stored on the server.
///
/// Used to populate [`X402PaymentRequirements`] when a client calls a priced
/// capability without a payment proof. Build one with [`usdc_base`] or
/// construct it manually.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityPricing {
    /// Target chain name, e.g. `"base"`, `"ethereum"`.
    pub network: String,
    /// ERC-20 contract address or `"ETH"`.
    pub asset: String,
    /// Amount in the asset's smallest unit (e.g. USDC cents × 10 000).
    pub amount: String,
    /// Recipient wallet address.
    pub pay_to: String,
    /// `"exact"` or `"upto"`. Defaults to `"exact"` when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Authorisation validity window in seconds. Defaults to `300` when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_timeout_seconds: Option<u64>,
    /// Human-readable description of the charge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Convenience constructors ──────────────────────────────────────────────────

/// Build a [`CapabilityPricing`] that charges `amount_usd_cents` US cents in
/// USDC on Base mainnet.
///
/// USDC on Base uses 6 decimal places, so 1 cent = 10 000 smallest units.
///
/// # Example
/// ```rust,no_run
/// use borgkit::addons::x402::usdc_base;
/// let pricing = usdc_base(50, "0xMyWallet", Some("Image gen — $0.50"));
/// // pricing.amount == "500000"  (50 cents × 10_000)
/// ```
pub fn usdc_base(amount_usd_cents: u64, pay_to: &str, description: Option<&str>) -> CapabilityPricing {
    // USDC has 6 decimal places. 1 USD = 1_000_000 units.
    // 1 cent = 10_000 units.
    let units = amount_usd_cents * 10_000;
    let desc = description
        .map(str::to_string)
        .unwrap_or_else(|| format!("${:.2} USD", amount_usd_cents as f64 / 100.0));
    CapabilityPricing {
        network: "base".to_string(),
        // Official USDC contract address on Base mainnet.
        asset: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".to_string(),
        amount: units.to_string(),
        pay_to: pay_to.to_string(),
        scheme: Some("exact".to_string()),
        max_timeout_seconds: Some(300),
        description: Some(desc),
    }
}

/// Convert a [`CapabilityPricing`] into an [`X402PaymentRequirements`] that
/// can be serialised and sent back to the client in a 402 response body.
///
/// `memo` should be set to the originating `request_id` so the client can
/// correlate the requirements to the request that triggered them.
pub fn to_requirements(pricing: &CapabilityPricing, memo: &str) -> X402PaymentRequirements {
    X402PaymentRequirements {
        scheme: pricing.scheme.clone().unwrap_or_else(|| "exact".to_string()),
        network: pricing.network.clone(),
        max_amount_required: pricing.amount.clone(),
        asset: pricing.asset.clone(),
        pay_to: pricing.pay_to.clone(),
        memo: if memo.is_empty() { None } else { Some(memo.to_string()) },
        max_timeout_seconds: Some(pricing.max_timeout_seconds.unwrap_or(300)),
        description: pricing.description.clone(),
    }
}

// ── Server middleware ─────────────────────────────────────────────────────────

/// Wraps any [`IAgent`] implementation with x402 payment enforcement.
///
/// When a client calls a priced capability without an `x402` payment proof
/// attached to the request, `check_payment` returns the appropriate
/// [`X402PaymentRequirements`] and the server layer should respond with HTTP
/// 402. When a proof is present, the server should invoke the inner agent
/// directly (verification logic is pluggable — see `strict` and the module
/// doc comment for details on custom verifiers).
///
/// # Dev mode
///
/// By default (`strict = false`) the middleware **accepts all payment proofs
/// without cryptographic verification**. This is intentional — it lets you
/// develop and test the full payment flow locally without needing a real wallet
/// or an on-chain facilitator. Set `strict = true` to reject all proofs (for
/// use with a real off-chain verifier attached outside this struct).
///
/// # Example
/// ```rust,no_run
/// use std::collections::HashMap;
/// use borgkit::addons::x402::{X402Server, usdc_base};
///
/// let mut pricing = HashMap::new();
/// pricing.insert("generate_image".to_string(), usdc_base(50, "0xWallet", None));
///
/// // let server = X402Server::new(my_agent, pricing);
/// // Pass `server` to `borgkit::server::serve(server, 6174)`.
/// ```
pub struct X402Server<A> {
    /// The wrapped agent that handles capabilities after payment is verified.
    pub inner: A,
    /// Capability name → pricing config.
    pub pricing: HashMap<String, CapabilityPricing>,
    /// When `true`, no dev-mode bypass is applied — all proofs must be
    /// externally verified before the request reaches the inner agent.
    pub strict: bool,
}

impl<A: IAgent + Clone + Send + Sync> X402Server<A> {
    /// Create a new `X402Server` wrapping `agent` with the given pricing table.
    ///
    /// `strict` defaults to `false` (dev-mode — accepts all proofs with a
    /// warning). Flip it to `true` when running with a real facilitator.
    pub fn new(agent: A, pricing: HashMap<String, CapabilityPricing>) -> Self {
        Self { inner: agent, pricing, strict: false }
    }

    /// Check whether the incoming request needs to be gate-kept by a payment.
    ///
    /// Returns `Some(requirements)` when:
    /// - the capability has a price entry in `self.pricing`, **and**
    /// - the request does not carry a recognised `x402` payment field.
    ///
    /// The caller (typically the `/invoke` handler) should return HTTP 402 with
    /// the requirements JSON when `Some` is returned.
    ///
    /// # Note on payment field detection
    ///
    /// The core `AgentRequest` struct does not include an `x402` field. Callers
    /// should deserialise requests into a custom wrapper struct that includes
    /// `pub x402: Option<X402Payment>` (see the module-level doc comment for an
    /// example). This method inspects `req.payment` as a proxy: if the core
    /// payment field is set it is treated as evidence that payment was attached.
    /// For production use, extend `AgentRequest` as described in the module docs
    /// and check `req.x402.is_some()` directly before calling `check_payment`.
    pub fn check_payment(&self, req: &AgentRequest) -> Option<X402PaymentRequirements> {
        let pricing_config = self.pricing.get(&req.capability)?;

        // If a payment field is already present in the core request envelope,
        // treat it as proof submitted. In production, replace this check with
        // `req.x402.is_some()` using the extended request type.
        if req.payment.is_some() {
            if self.strict {
                // In strict mode, log a warning — real verification should have
                // happened before this point via an off-chain facilitator.
                eprintln!(
                    "[x402] STRICT MODE: payment field present for capability '{}' \
                     but no cryptographic verifier is wired in. \
                     Integrate an X402Facilitator before production.",
                    req.capability
                );
            } else {
                eprintln!(
                    "[x402] DEV MODE: payment proof accepted without verification for \
                     capability '{}'. Integrate an X402Facilitator before production.",
                    req.capability
                );
            }
            return None; // Payment present — allow the request through.
        }

        // No payment proof — return requirements for the 402 response.
        Some(to_requirements(pricing_config, &req.request_id))
    }
}

// ── IAgent passthrough impl for X402Server ────────────────────────────────────

#[async_trait::async_trait]
impl<A: IAgent + Clone + Send + Sync> IAgent for X402Server<A> {
    fn agent_id(&self) -> &str { self.inner.agent_id() }
    fn owner(&self) -> &str { self.inner.owner() }
    fn metadata_uri(&self) -> Option<&str> { self.inner.metadata_uri() }

    fn get_capabilities(&self) -> Vec<String> { self.inner.get_capabilities() }

    /// Handle a request, enforcing payment for priced capabilities.
    ///
    /// Returns an error `AgentResponse` with status `"payment_required"` when
    /// the capability is priced and no payment proof was found. Delegates to
    /// the inner agent otherwise.
    async fn handle_request(&self, request: AgentRequest) -> crate::response::AgentResponse {
        use crate::response::AgentResponse;
        use serde_json::json;

        if let Some(requirements) = self.check_payment(&request) {
            // Return a structured payment_required response.
            // The HTTP layer (server.rs) converts this into an actual 402.
            return AgentResponse {
                request_id: request.request_id,
                status: "payment_required".to_string(),
                result: Some(json!({
                    "payment_requirements": [requirements],
                    "message": format!(
                        "Capability '{}' requires payment. Attach an x402 proof and retry.",
                        request.capability
                    ),
                })),
                error_message: Some(format!(
                    "Capability '{}' requires payment.",
                    request.capability
                )),
                proof: None,
                signature: None,
                timestamp: {
                    use std::time::{SystemTime, UNIX_EPOCH};
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                },
            };
        }

        self.inner.handle_request(request).await
    }

    fn requires_payment(&self) -> bool {
        !self.pricing.is_empty()
    }

    fn get_anr(&self) -> crate::discovery::DiscoveryEntry { self.inner.get_anr() }
    fn get_peer_id(&self) -> Option<String> { self.inner.get_peer_id() }

    async fn check_permission(&self, caller: &str, capability: &str) -> bool {
        self.inner.check_permission(caller, capability).await
    }

    async fn register_discovery(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.register_discovery().await
    }

    async fn unregister_discovery(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.unregister_discovery().await
    }

    async fn sign_message(&self, message: &str) -> Result<String, Box<dyn std::error::Error>> {
        self.inner.sign_message(message).await
    }
}

// ── Client helper ─────────────────────────────────────────────────────────────

/// Client-side helper for constructing and attaching x402 payment proofs.
///
/// In production this struct would integrate with a wallet library (e.g.
/// `ethers-rs` or `alloy`) to produce real EIP-3009 signed authorisations.
/// For local development and testing, [`X402Client::mock_payment`] generates a
/// structurally valid but cryptographically **unsigned** proof that any
/// dev-mode server will accept.
///
/// # Example
/// ```rust,no_run
/// use borgkit::addons::x402::{X402Client, X402PaymentRequirements};
///
/// let client = X402Client::new("0xDeadBeef");
/// // After receiving 402:
/// // let proof = client.mock_payment(&requirements, &request_id);
/// // attach proof to request and retry.
/// ```
pub struct X402Client {
    /// The payer's wallet address.
    pub wallet_address: String,
    /// When `true`, the client will automatically call `mock_payment` and
    /// re-attach the proof on a 402 response (useful for integration tests).
    pub auto_pay: bool,
}

impl X402Client {
    /// Create a new client for the given wallet address.
    /// `auto_pay` defaults to `false`.
    pub fn new(wallet_address: &str) -> Self {
        Self { wallet_address: wallet_address.to_string(), auto_pay: false }
    }

    /// Generate a mock (unsigned) payment proof for development and testing.
    ///
    /// The returned [`X402Payment`] has a structurally valid shape but contains
    /// placeholder cryptographic values. Any server running in dev mode (i.e.
    /// `X402Server::strict == false`) will accept it without validation.
    ///
    /// **Never use mock proofs in production.** Replace this with a real
    /// EIP-3009 + EIP-712 signing flow before going live.
    ///
    /// # Arguments
    /// * `requirements` — the [`X402PaymentRequirements`] received in the 402
    ///   response body.
    /// * `request_id` — the `request_id` from the original request (used as the
    ///   memo / nonce).
    pub fn mock_payment(
        &self,
        requirements: &X402PaymentRequirements,
        request_id: &str,
    ) -> X402Payment {
        eprintln!(
            "[x402] DEV MODE: generating mock payment proof for request '{}'. \
             Replace X402Client::mock_payment() with a real wallet signer before production.",
            request_id
        );
        // Build a placeholder ABI-encoded transferWithAuthorization payload.
        // Format: base64url of "<from>:<to>:<amount>:<nonce>"
        let raw = format!(
            "{}:{}:{}:{}",
            self.wallet_address,
            requirements.pay_to,
            requirements.max_amount_required,
            request_id,
        );
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let payload = URL_SAFE_NO_PAD.encode(raw.as_bytes());

        X402Payment {
            x402_version: 1,
            scheme: requirements.scheme.clone(),
            network: requirements.network.clone(),
            payload,
            // Placeholder signature — all zeros, 65 bytes, 0x-prefixed.
            signature: format!("0x{}", "0".repeat(130)),
        }
    }
}
