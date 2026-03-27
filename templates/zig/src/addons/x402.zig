//! x402 micropayment protocol — Zig implementation for Borgkit agents.
//!
//! The x402 protocol (https://x402.org) lets any HTTP endpoint gate access
//! behind an on-chain micropayment. The flow is:
//!
//!   1. Client calls POST /invoke (no payment attached).
//!   2. Server responds HTTP 402 with a JSON body containing
//!      `X402PaymentRequirements` describing how much, in what token, on
//!      which network, and to which wallet.
//!   3. Client submits an EIP-3009 `transferWithAuthorization` signed over
//!      the requirements, encoding it as an `X402Payment`.
//!   4. Client re-calls POST /invoke, this time attaching the payment proof
//!      in the `AgentRequest.payment` field.
//!   5. Server (optionally via a facilitator) verifies and settles on-chain,
//!      then proceeds with the request.
//!
//! ## Quick-start usage in a Borgkit agent
//!
//! ```zig
//! const x402 = @import("addons/x402.zig");
//!
//! pub const PRICING = x402.usdcBase(10, "0xYourWallet", "My AI service", allocator)
//!     catch @panic("usdcBase failed");
//!
//! pub fn requiresPayment(self: *const @This()) bool {
//!     _ = self;
//!     return true;
//! }
//!
//! // In handleRequest, after the server lets the request through:
//! pub fn handleRequest(self: *@This(), req: types.AgentRequest) types.AgentResponse {
//!     _ = req;
//!     return types.AgentResponse.success(req.request_id, "\"ok\"");
//! }
//! ```
//!
//! The `server.zig` 402-gating block checks `requiresPayment()` at comptime
//! and rejects requests that lack `AgentRequest.payment`.  Wire a richer
//! 402 body by calling `paymentRequiredBody()` from your own route handler
//! instead of relying on the default stub response.
//!
//! ## DEV MODE
//!
//! Call `mockPayment()` to generate a syntactically valid (but on-chain
//! unverifiable) `X402Payment` for local testing without a real wallet.
//!
//! Reference: https://x402.org  |  https://github.com/coinbase/x402

const std = @import("std");

// ── USDC on Base contract address ─────────────────────────────────────────────

/// ERC-20 address of USDC on Base mainnet.
pub const USDC_BASE_ADDRESS: []const u8 = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

// ── Types ─────────────────────────────────────────────────────────────────────

/// Payment requirements sent to the client when HTTP 402 is returned.
///
/// Mirrors the TypeScript `X402PaymentRequirements` interface at
/// `templates/typescript/addons/x402/types.ts`.
pub const X402PaymentRequirements = struct {
    /// Payment scheme: "exact" (fixed amount) or "upto" (at-most amount).
    scheme: []const u8,
    /// Chain name: "base", "ethereum", "polygon", etc.
    network: []const u8,
    /// Required payment amount expressed in the token's smallest unit.
    /// USDC uses 6 decimal places; ETH/wei uses 18.
    max_amount_required: []const u8,
    /// ERC-20 contract address (e.g. USDC on Base) or "ETH" for native ether.
    asset: []const u8,
    /// Recipient wallet address that will receive the payment.
    pay_to: []const u8,
    /// Correlation ID — callers typically echo back the AgentRequest.request_id.
    memo: ?[]const u8 = null,
    /// How many seconds the signed authorization is valid (default 300 = 5 min).
    max_timeout_seconds: ?u64 = 300,
    /// Human-readable description of what is being charged for.
    description: ?[]const u8 = null,

    /// Serialize to a JSON string.  Caller owns the returned slice and must
    /// free it with `allocator.free()`.
    pub fn toJson(self: X402PaymentRequirements, allocator: std.mem.Allocator) ![]const u8 {
        var list = std.ArrayList(u8).init(allocator);
        errdefer list.deinit();
        const writer = list.writer();

        try writer.writeAll("{");

        // Required fields
        try writer.print("\"scheme\":\"{s}\"", .{self.scheme});
        try writer.print(",\"network\":\"{s}\"", .{self.network});
        try writer.print(",\"maxAmountRequired\":\"{s}\"", .{self.max_amount_required});
        try writer.print(",\"asset\":\"{s}\"", .{self.asset});
        try writer.print(",\"payTo\":\"{s}\"", .{self.pay_to});

        // Optional fields
        if (self.memo) |m| {
            try writer.print(",\"memo\":\"{s}\"", .{m});
        } else {
            try writer.writeAll(",\"memo\":null");
        }

        if (self.max_timeout_seconds) |t| {
            try writer.print(",\"maxTimeoutSeconds\":{d}", .{t});
        } else {
            try writer.writeAll(",\"maxTimeoutSeconds\":300");
        }

        if (self.description) |d| {
            try writer.print(",\"description\":\"{s}\"", .{d});
        } else {
            try writer.writeAll(",\"description\":null");
        }

        try writer.writeAll("}");

        return list.toOwnedSlice();
    }
};

/// Payment proof submitted by the client inside `AgentRequest.payment`.
///
/// The `payload` field carries a Base64url-encoded EIP-3009
/// `transferWithAuthorization` authorization, and `signature` is the
/// outer EIP-712 envelope signed over (scheme + network + payload).
pub const X402Payment = struct {
    /// x402 protocol version — currently always 1.
    x402_version: u32 = 1,
    /// Payment scheme matching the requirements that triggered the 402.
    scheme: []const u8,
    /// Chain name matching the requirements.
    network: []const u8,
    /// Base64url-encoded signed EIP-3009 transferWithAuthorization proof.
    payload: []const u8,
    /// Outer EIP-712 signature covering scheme + network + payload.
    signature: []const u8,
};

/// Settlement receipt returned by a facilitator service after it attempts to
/// settle the payment on-chain.
pub const X402Receipt = struct {
    /// Whether the on-chain settlement succeeded.
    success: bool,
    /// Transaction hash of the settled transfer (null if not yet settled).
    transaction_hash: ?[]const u8 = null,
    /// Human-readable failure reason (null on success).
    error_reason: ?[]const u8 = null,
};

/// Per-capability pricing configuration stored server-side.
///
/// Pass one of these to `toRequirements()` when you need to build the 402
/// response body, or use `usdcBase()` to create a pre-filled USDC/Base entry.
pub const CapabilityPricing = struct {
    /// Chain name ("base", "ethereum", …).
    network: []const u8,
    /// Token contract address or "ETH".
    asset: []const u8,
    /// Required payment in smallest units (e.g. "10000" = $0.01 USDC).
    amount: []const u8,
    /// Recipient wallet address.
    pay_to: []const u8,
    /// "exact" or "upto" (default "exact").
    scheme: []const u8 = "exact",
    /// Authorization TTL in seconds (default 300).
    max_timeout_seconds: u64 = 300,
    /// Optional human-readable description of the charge.
    description: ?[]const u8 = null,
};

// ── Convenience helpers ───────────────────────────────────────────────────────

/// Build a `CapabilityPricing` for a USDC-on-Base charge of `amount_usd_cents`
/// US cents.
///
/// Example: `usdcBase(10, "0xWallet", "My AI query", allocator)` charges
/// $0.10 USDC on Base.
///
/// The `description` field defaults to "$X.XX USD" when null.
/// Caller must free the `description` string via `allocator.free()` if a
/// default was generated (detect by checking whether it equals `description`).
///
/// Note: because CapabilityPricing holds slices (not owned allocations), the
/// returned struct is valid only as long as `allocator` and `pay_to`/
/// `description` strings remain alive.
pub fn usdcBase(
    amount_usd_cents: u64,
    pay_to: []const u8,
    description: ?[]const u8,
    allocator: std.mem.Allocator,
) !CapabilityPricing {
    // cents → 6-decimal USDC units: 1 cent = 10_000 base units.
    const usdc_amount = amount_usd_cents * 10_000;

    // Build the amount string.
    const amount_str = try std.fmt.allocPrint(allocator, "{d}", .{usdc_amount});
    errdefer allocator.free(amount_str);

    // Build description string if not provided.
    const desc: ?[]const u8 = if (description) |d|
        d
    else blk: {
        const dollars = amount_usd_cents / 100;
        const cents   = amount_usd_cents % 100;
        break :blk try std.fmt.allocPrint(
            allocator,
            "${d}.{d:0>2} USD",
            .{ dollars, cents },
        );
    };

    return CapabilityPricing{
        .network              = "base",
        .asset                = USDC_BASE_ADDRESS,
        .amount               = amount_str,
        .pay_to               = pay_to,
        .scheme               = "exact",
        .max_timeout_seconds  = 300,
        .description          = desc,
    };
}

/// Convert a `CapabilityPricing` to an `X402PaymentRequirements` suitable for
/// sending to the client.
///
/// `memo` is typically the `AgentRequest.request_id` so the client can
/// correlate the payment proof with the original request.
pub fn toRequirements(pricing: CapabilityPricing, memo: []const u8) X402PaymentRequirements {
    return X402PaymentRequirements{
        .scheme               = pricing.scheme,
        .network              = pricing.network,
        .max_amount_required  = pricing.amount,
        .asset                = pricing.asset,
        .pay_to               = pricing.pay_to,
        .memo                 = if (memo.len > 0) memo else null,
        .max_timeout_seconds  = pricing.max_timeout_seconds,
        .description          = pricing.description,
    };
}

// ── 402 response builder ──────────────────────────────────────────────────────

/// Build the HTTP 402 response body JSON for a capability that requires payment.
///
/// Returns a JSON object of the form:
/// ```json
/// {
///   "error": "payment required",
///   "code": "402",
///   "capability": "<capability>",
///   "requirements": { <X402PaymentRequirements> }
/// }
/// ```
///
/// Caller owns the returned slice and must free it with `allocator.free()`.
pub fn paymentRequiredBody(
    requirements: X402PaymentRequirements,
    capability: []const u8,
    allocator: std.mem.Allocator,
) ![]const u8 {
    const req_json = try requirements.toJson(allocator);
    defer allocator.free(req_json);

    return std.fmt.allocPrint(
        allocator,
        "{{\"error\":\"payment required\",\"code\":\"402\",\"capability\":\"{s}\",\"requirements\":{s}}}",
        .{ capability, req_json },
    );
}

// ── DEV MODE mock payment ─────────────────────────────────────────────────────

/// Create a mock `X402Payment` for local development and testing (DEV MODE).
///
/// The returned payment has a structurally valid shape but carries placeholder
/// `payload` and `signature` values.  It will pass JSON parsing on the server
/// side but will NOT verify against any on-chain state — use only in dev/test
/// environments where payment verification is disabled or mocked.
///
/// `request_id` is used as the memo/correlation value embedded in the payload.
/// Caller owns all string fields in the returned struct; they are allocated
/// from `allocator` and must be freed individually or via an arena.
pub fn mockPayment(
    requirements: X402PaymentRequirements,
    request_id: []const u8,
    allocator: std.mem.Allocator,
) !X402Payment {
    // Build a fake Base64url payload that encodes the key parameters so
    // server-side log output is informative during testing.
    const raw = try std.fmt.allocPrint(
        allocator,
        "MOCK|to={s}|amount={s}|network={s}|memo={s}",
        .{
            requirements.pay_to,
            requirements.max_amount_required,
            requirements.network,
            request_id,
        },
    );
    defer allocator.free(raw);

    // Base64-encode the raw payload (standard alphabet; url-safe variant is
    // preferred by the spec but both work for testing purposes).
    const encoded_len = std.base64.standard.Encoder.calcSize(raw.len);
    const payload = try allocator.alloc(u8, encoded_len);
    _ = std.base64.standard.Encoder.encode(payload, raw);

    // Placeholder outer signature — 65 zero bytes hex-encoded, resembling a
    // real EIP-712 signature in length (130 hex chars + "0x" prefix = 132).
    const signature = try allocator.dupe(
        u8,
        "0x" ++ "00" ** 65,
    );

    const scheme_copy  = try allocator.dupe(u8, requirements.scheme);
    const network_copy = try allocator.dupe(u8, requirements.network);

    return X402Payment{
        .x402_version = 1,
        .scheme       = scheme_copy,
        .network      = network_copy,
        .payload      = payload,
        .signature    = signature,
    };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test "usdcBase produces correct amount string" {
    const allocator = std.testing.allocator;

    var pricing = try usdcBase(10, "0xWallet", null, allocator);
    defer allocator.free(pricing.amount);
    defer if (pricing.description) |d| allocator.free(d);

    // 10 cents × 10_000 = 100_000 base units
    try std.testing.expectEqualStrings("100000", pricing.amount);
    try std.testing.expectEqualStrings("base", pricing.network);
    try std.testing.expectEqualStrings(USDC_BASE_ADDRESS, pricing.asset);
}

test "toRequirements maps all fields" {
    const pricing = CapabilityPricing{
        .network     = "base",
        .asset       = USDC_BASE_ADDRESS,
        .amount      = "50000",
        .pay_to      = "0xRecipient",
        .scheme      = "exact",
        .description = "Test charge",
    };

    const req = toRequirements(pricing, "req-123");

    try std.testing.expectEqualStrings("exact",         req.scheme);
    try std.testing.expectEqualStrings("base",          req.network);
    try std.testing.expectEqualStrings("50000",         req.max_amount_required);
    try std.testing.expectEqualStrings(USDC_BASE_ADDRESS, req.asset);
    try std.testing.expectEqualStrings("0xRecipient",   req.pay_to);
    try std.testing.expectEqualStrings("req-123",       req.memo.?);
}

test "toJson round-trip contains required keys" {
    const allocator = std.testing.allocator;

    const req = X402PaymentRequirements{
        .scheme               = "exact",
        .network              = "base",
        .max_amount_required  = "100000",
        .asset                = USDC_BASE_ADDRESS,
        .pay_to               = "0xWallet",
        .memo                 = "req-abc",
        .max_timeout_seconds  = 300,
        .description          = "0.10 USD test",
    };

    const json = try req.toJson(allocator);
    defer allocator.free(json);

    try std.testing.expect(std.mem.indexOf(u8, json, "\"scheme\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"maxAmountRequired\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"payTo\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "exact") != null);
}

test "paymentRequiredBody wraps requirements" {
    const allocator = std.testing.allocator;

    const req = X402PaymentRequirements{
        .scheme               = "exact",
        .network              = "base",
        .max_amount_required  = "100000",
        .asset                = USDC_BASE_ADDRESS,
        .pay_to               = "0xWallet",
    };

    const body = try paymentRequiredBody(req, "myCapability", allocator);
    defer allocator.free(body);

    try std.testing.expect(std.mem.indexOf(u8, body, "payment required") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "402") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "myCapability") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "requirements") != null);
}

test "mockPayment produces non-empty payload and signature" {
    const allocator = std.testing.allocator;

    const req = X402PaymentRequirements{
        .scheme               = "exact",
        .network              = "base",
        .max_amount_required  = "100000",
        .asset                = USDC_BASE_ADDRESS,
        .pay_to               = "0xWallet",
    };

    const payment = try mockPayment(req, "req-dev-001", allocator);
    defer {
        allocator.free(payment.payload);
        allocator.free(payment.signature);
        allocator.free(payment.scheme);
        allocator.free(payment.network);
    }

    try std.testing.expect(payment.payload.len > 0);
    try std.testing.expect(payment.signature.len > 0);
    try std.testing.expectEqual(@as(u32, 1), payment.x402_version);
    try std.testing.expectEqualStrings("exact", payment.scheme);
}
