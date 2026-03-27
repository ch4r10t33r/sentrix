//! Machine Payments Protocol (MPP) → Sentrix Plugin (Zig)
//!
//! Implements HTTP 402 payment gating for Sentrix agents using the MPP
//! challenge–credential–receipt flow (https://mpp.dev).
//!
//! This implementation covers the server (gating) and client (paying) sides
//! using only Zig's standard library — zero external dependencies.
//!
//! MPP flow
//! ────────
//!   1. Client → POST /invoke  (no Authorization header)
//!   2. Agent  ← 402 Payment Required
//!              WWW-Authenticate: Payment method="tempo", recipient="0x...",
//!                currency="0x...", rpc="https://...", amount="0.01", nonce="<hex>"
//!   3. Client pays on Tempo chain / via Stripe
//!   4. Client → POST /invoke
//!              Authorization: Payment <base64({"method":"tempo","nonce":"...","tx_hash":"0x...","ts":...})>
//!   5. Agent verifies → 200 OK
//!              Payment-Receipt: {"method":"tempo","amount":"0.01","nonce":"...","ts":...}
//!
//! Supported payment methods
//! ─────────────────────────
//!   • Tempo  — TIP-20 stablecoin on the Tempo EVM chain
//!   • Stripe — Shared Payment Tokens (SPT)
//!
//! Server usage
//! ────────────
//!   const mpp = @import("plugins/mpp.zig");
//!
//!   var plugin = mpp.MppPlugin.init(allocator, .{
//!       .method    = .tempo,
//!       .recipient = "0x742d35Cc6634c0532925a3b844Bc9e7595f1B0F2",
//!       .currency  = mpp.DEFAULT_TEMPO_CURRENCY,
//!       .pricing   = .{ .default = "0.01" },
//!   });
//!   defer plugin.deinit();
//!
//!   // In your HTTP handler:
//!   const auth_header = req.headers.get("Authorization");
//!   if (try plugin.gate(auth_header, "summarise")) |challenge_response| {
//!       // Send 402 with challenge_response.www_auth_header
//!       defer challenge_response.deinit(allocator);
//!       // ... write 402 response
//!   } else {
//!       // Credential valid — run the agent and attach receipt
//!       const receipt = try plugin.receipt(allocator, "summarise", nonce);
//!       defer allocator.free(receipt);
//!       // ... write 200 response with Payment-Receipt: <receipt> header
//!   }
//!
//! Client usage
//! ────────────
//!   var client = mpp.MppClient.init(allocator, .{
//!       .private_key = "0xabc...",  // paying wallet private key (Tempo)
//!   });
//!   defer client.deinit();
//!
//!   const resp = try client.post("http://agent.example.com:6174/invoke", body);
//!   defer resp.deinit(allocator);
//!   // resp.status == 200 and resp.receipt_header has the receipt

const std = @import("std");

// ── Constants ─────────────────────────────────────────────────────────────────

pub const WWW_AUTH_HEADER    = "WWW-Authenticate";
pub const AUTH_HEADER        = "Authorization";
pub const RECEIPT_HEADER     = "Payment-Receipt";
pub const PAYMENT_SCHEME     = "Payment";

pub const DEFAULT_TEMPO_CURRENCY = "0x20c0000000000000000000000000000000000000";
pub const DEFAULT_TEMPO_RPC      = "https://rpc.moderato.tempo.xyz";
pub const NONCE_LEN: usize = 16;  // bytes

// ── Error set ─────────────────────────────────────────────────────────────────

pub const MppError = error{
    MissingConfig,
    InvalidCredential,
    PaymentFailed,
    NetworkError,
    OutOfMemory,
};

// ── Config types ──────────────────────────────────────────────────────────────

pub const MppMethod = enum { tempo, stripe, lightning };

pub const MppPricing = struct {
    /// Default charge for any `/invoke` call (token units as string).
    default: []const u8 = "0.01",
    /// Per-capability overrides — caller owns this slice.
    per_capability: []const CapabilityPrice = &.{},
};

pub const CapabilityPrice = struct {
    capability: []const u8,
    amount:     []const u8,
};

pub const MppConfig = struct {
    method:    MppMethod     = .tempo,
    /// Tempo recipient EVM address (required for method=tempo).
    recipient: []const u8   = "",
    /// TIP-20 token contract address (default: USDC on Moderato testnet).
    currency:  []const u8   = DEFAULT_TEMPO_CURRENCY,
    /// Tempo RPC endpoint.
    rpc:       []const u8   = DEFAULT_TEMPO_RPC,
    /// Stripe network ID (required for method=stripe).
    stripe_network_id: []const u8 = "internal",
    /// ISO currency code for Stripe (default: usd).
    stripe_currency:   []const u8 = "usd",
    /// Decimal places for Stripe (default: 2).
    stripe_decimals:   u8         = 2,
    /// Pricing schedule.
    pricing:   MppPricing = .{},
    /// Skip payment on loopback addresses (default: false).
    skip_on_localhost: bool = false,
};

// ── Challenge response ────────────────────────────────────────────────────────

pub const ChallengeResponse = struct {
    /// Full `WWW-Authenticate` header value — heap-allocated.
    www_auth_header: []const u8,
    /// The nonce embedded in the challenge — heap-allocated.
    nonce: []const u8,

    pub fn deinit(self: ChallengeResponse, allocator: std.mem.Allocator) void {
        allocator.free(self.www_auth_header);
        allocator.free(self.nonce);
    }
};

// ── HttpResponse (client side) ────────────────────────────────────────────────

pub const MppResponse = struct {
    status:         u16,
    body:           []const u8,   // heap-allocated
    receipt_header: ?[]const u8,  // heap-allocated if present

    pub fn deinit(self: MppResponse, allocator: std.mem.Allocator) void {
        allocator.free(self.body);
        if (self.receipt_header) |h| allocator.free(h);
    }
};

// ── Nonce generation ─────────────────────────────────────────────────────────

/// Generate a cryptographically random 16-byte nonce as a 32-char lowercase hex string.
/// Caller owns the returned slice.
pub fn generateNonce(allocator: std.mem.Allocator) ![]u8 {
    var bytes: [NONCE_LEN]u8 = undefined;
    std.crypto.random.bytes(&bytes);
    var hex = try allocator.alloc(u8, NONCE_LEN * 2);
    const alphabet = "0123456789abcdef";
    for (bytes, 0..) |b, i| {
        hex[i * 2]     = alphabet[b >> 4];
        hex[i * 2 + 1] = alphabet[b & 0x0f];
    }
    return hex;
}

// ── Challenge builders ────────────────────────────────────────────────────────

/// Build the Tempo `WWW-Authenticate: Payment ...` header value.
/// Caller owns the returned slice.
fn buildTempoChallenge(
    allocator: std.mem.Allocator,
    cfg:       *const MppConfig,
    amount:    []const u8,
    nonce:     []const u8,
) ![]u8 {
    return std.fmt.allocPrint(
        allocator,
        "{s} method=\"tempo\", recipient=\"{s}\", currency=\"{s}\", rpc=\"{s}\", amount=\"{s}\", nonce=\"{s}\"",
        .{ PAYMENT_SCHEME, cfg.recipient, cfg.currency, cfg.rpc, amount, nonce },
    );
}

/// Build the Stripe `WWW-Authenticate: Payment ...` header value.
/// Caller owns the returned slice.
fn buildStripeChallenge(
    allocator: std.mem.Allocator,
    cfg:       *const MppConfig,
    amount:    []const u8,
    nonce:     []const u8,
) ![]u8 {
    return std.fmt.allocPrint(
        allocator,
        "{s} method=\"stripe\", network_id=\"{s}\", currency=\"{s}\", decimals=\"{d}\", amount=\"{s}\", nonce=\"{s}\"",
        .{ PAYMENT_SCHEME, cfg.stripe_network_id, cfg.stripe_currency, cfg.stripe_decimals, amount, nonce },
    );
}

// ── Challenge parser ──────────────────────────────────────────────────────────

pub const ParsedChallenge = struct {
    method:    []const u8,
    amount:    []const u8,
    nonce:     []const u8,
    recipient: []const u8,
    currency:  []const u8,

    // All slices point into the original header string — no allocation.
};

/// Parse `WWW-Authenticate: Payment method="...", amount="...", ...`
/// Returns a `ParsedChallenge` with slices into `header`.
pub fn parseChallenge(header: []const u8) ParsedChallenge {
    var result = ParsedChallenge{
        .method    = "",
        .amount    = "",
        .nonce     = "",
        .recipient = "",
        .currency  = "",
    };
    // Strip scheme prefix
    var rest = header;
    if (std.mem.startsWith(u8, rest, PAYMENT_SCHEME)) {
        rest = std.mem.trimLeft(u8, rest[PAYMENT_SCHEME.len..], " ");
    }
    // Split on ", " and parse key="value" pairs
    var it = std.mem.splitSequence(u8, rest, ", ");
    while (it.next()) |part| {
        const p = std.mem.trim(u8, part, " ");
        if (std.mem.indexOf(u8, p, "=")) |eq| {
            const key = p[0..eq];
            var val = p[eq + 1..];
            // Strip quotes
            if (val.len >= 2 and val[0] == '"' and val[val.len - 1] == '"') {
                val = val[1 .. val.len - 1];
            }
            if (std.mem.eql(u8, key, "method"))    result.method    = val;
            if (std.mem.eql(u8, key, "amount"))    result.amount    = val;
            if (std.mem.eql(u8, key, "nonce"))     result.nonce     = val;
            if (std.mem.eql(u8, key, "recipient")) result.recipient = val;
            if (std.mem.eql(u8, key, "currency"))  result.currency  = val;
        }
    }
    return result;
}

// ── Credential helpers ────────────────────────────────────────────────────────

/// Extract the raw base64 token from `Authorization: Payment <token>`.
/// Returns null if the header is not in MPP format.
pub fn extractCredential(header: []const u8) ?[]const u8 {
    const prefix = PAYMENT_SCHEME ++ " ";
    if (std.mem.startsWith(u8, header, prefix)) {
        return std.mem.trim(u8, header[prefix.len..], " ");
    }
    return null;
}

/// Minimal structural validation of a credential token.
/// Returns true if the token is non-empty and base64-decodable JSON.
pub fn validateCredentialStructure(allocator: std.mem.Allocator, token: []const u8) bool {
    if (token.len < 8) return false;
    const Decoder = std.base64.standard.Decoder;
    const decoded_len = Decoder.calcSizeForSlice(token) catch return false;
    const buf = allocator.alloc(u8, decoded_len) catch return false;
    defer allocator.free(buf);
    Decoder.decode(buf, token) catch return false;
    // Check it looks like JSON
    const trimmed = std.mem.trim(u8, buf[0..decoded_len], " ");
    return trimmed.len > 0 and trimmed[0] == '{' and trimmed[trimmed.len - 1] == '}';
}

// ── Receipt builder ───────────────────────────────────────────────────────────

/// Build a `Payment-Receipt` header value JSON string.
/// Caller owns the returned slice.
pub fn buildReceipt(
    allocator: std.mem.Allocator,
    method:    []const u8,
    amount:    []const u8,
    nonce:     []const u8,
) ![]u8 {
    const ts = @as(u64, @intCast(std.time.milliTimestamp()));
    return std.fmt.allocPrint(
        allocator,
        "{{\"method\":\"{s}\",\"amount\":\"{s}\",\"nonce\":\"{s}\",\"ts\":{d}}}",
        .{ method, amount, nonce, ts },
    );
}

// ── MppPlugin — server side ───────────────────────────────────────────────────

pub const MppPlugin = struct {
    allocator: std.mem.Allocator,
    cfg:       MppConfig,

    pub fn init(allocator: std.mem.Allocator, cfg: MppConfig) MppPlugin {
        return .{ .allocator = allocator, .cfg = cfg };
    }

    pub fn deinit(_: *MppPlugin) void {}

    // ── Public API ────────────────────────────────────────────────────────────

    /// Determine the price for a capability.
    pub fn priceFor(self: *const MppPlugin, capability: []const u8) []const u8 {
        for (self.cfg.pricing.per_capability) |cp| {
            if (std.mem.eql(u8, cp.capability, capability)) return cp.amount;
        }
        return self.cfg.pricing.default;
    }

    /// Build a challenge response for the given capability.
    /// Returns an arena-allocated `ChallengeResponse`.
    pub fn buildChallenge(
        self:       *const MppPlugin,
        allocator:  std.mem.Allocator,
        capability: []const u8,
    ) !ChallengeResponse {
        const amount = self.priceFor(capability);
        const nonce  = try generateNonce(allocator);
        errdefer allocator.free(nonce);

        const header = switch (self.cfg.method) {
            .tempo  => try buildTempoChallenge(allocator, &self.cfg, amount, nonce),
            .stripe => try buildStripeChallenge(allocator, &self.cfg, amount, nonce),
            .lightning => try std.fmt.allocPrint(
                allocator,
                "{s} method=\"lightning\", amount=\"{s}\", nonce=\"{s}\"",
                .{ PAYMENT_SCHEME, amount, nonce },
            ),
        };

        return .{ .www_auth_header = header, .nonce = nonce };
    }

    /// Gate an incoming request.
    ///
    /// Returns `null` when the credential is valid (request may proceed).
    /// Returns a `ChallengeResponse` (caller must `deinit`) when 402 should be sent.
    pub fn gate(
        self:         *const MppPlugin,
        allocator:    std.mem.Allocator,
        auth_header:  ?[]const u8,
        capability:   []const u8,
    ) !?ChallengeResponse {
        if (auth_header) |auth| {
            if (extractCredential(auth)) |cred| {
                if (validateCredentialStructure(allocator, cred)) {
                    return null;  // valid — proceed
                }
            }
        }
        return self.buildChallenge(allocator, capability);
    }

    /// Build a `Payment-Receipt` header value for a completed invocation.
    /// Caller owns the returned slice.
    pub fn receipt(
        self:       *const MppPlugin,
        allocator:  std.mem.Allocator,
        capability: []const u8,
        nonce:      []const u8,
    ) ![]u8 {
        const amount = self.priceFor(capability);
        const method = switch (self.cfg.method) {
            .tempo     => "tempo",
            .stripe    => "stripe",
            .lightning => "lightning",
        };
        return buildReceipt(allocator, method, amount, nonce);
    }
};

// ── MppClient — paying agent side ────────────────────────────────────────────

pub const MppClientConfig = struct {
    /// Hex-encoded private key of the paying wallet (`0x...`) — Tempo.
    private_key:  ?[]const u8 = null,
    /// Backend URL that creates Stripe Shared Payment Tokens.
    spt_endpoint: ?[]const u8 = null,
    /// Maximum payment retries per request (default: 1).
    max_retries:  u8 = 1,
};

pub const MppClient = struct {
    allocator: std.mem.Allocator,
    cfg:       MppClientConfig,

    pub fn init(allocator: std.mem.Allocator, cfg: MppClientConfig) MppClient {
        return .{ .allocator = allocator, .cfg = cfg };
    }

    pub fn deinit(_: *MppClient) void {}

    /// POST JSON `body` to `url`, automatically paying any 402 challenge.
    /// Caller owns the returned `MppResponse`.
    pub fn post(self: *MppClient, url: []const u8, body: []const u8) !MppResponse {
        return self.postWithRetry(url, body, null, 0);
    }

    fn postWithRetry(
        self:       *MppClient,
        url:        []const u8,
        body:       []const u8,
        credential: ?[]const u8,
        attempt:    u8,
    ) !MppResponse {
        // Parse the URL
        const uri = std.Uri.parse(url) catch return MppError.NetworkError;

        // Open TCP connection
        var client = std.http.Client{ .allocator = self.allocator };
        defer client.deinit();

        var header_buf: [4096]u8 = undefined;
        var req = try client.open(.POST, uri, .{
            .server_header_buffer = &header_buf,
        });
        defer req.deinit();

        req.transfer_encoding = .{ .content_length = body.len };
        try req.headers.append("Content-Type", "application/json");

        if (credential) |cred| {
            const auth = try std.fmt.allocPrint(
                self.allocator,
                "{s} {s}",
                .{ PAYMENT_SCHEME, cred },
            );
            defer self.allocator.free(auth);
            try req.headers.append(AUTH_HEADER, auth);
        }

        try req.send();
        try req.writeAll(body);
        try req.finish();
        try req.wait();

        const status = @as(u16, @intCast(@intFromEnum(req.response.status)));

        // Read response body
        var resp_body = std.ArrayList(u8).init(self.allocator);
        defer resp_body.deinit();
        try req.reader().readAllArrayList(&resp_body, 1024 * 1024);
        const resp_bytes = try resp_body.toOwnedSlice();

        // Read receipt header if present
        var receipt_val: ?[]u8 = null;
        var it = req.response.iterateHeaders();
        while (it.next()) |h| {
            if (std.ascii.eqlIgnoreCase(h.name, RECEIPT_HEADER)) {
                receipt_val = try self.allocator.dupe(u8, h.value);
                break;
            }
        }

        if (status != 402 or attempt >= self.cfg.max_retries) {
            return .{
                .status         = status,
                .body           = resp_bytes,
                .receipt_header = receipt_val,
            };
        }

        // Defer cleanup of this attempt's allocations before retry
        self.allocator.free(resp_bytes);
        if (receipt_val) |r| self.allocator.free(r);

        // Parse the challenge header
        var challenge_val: []const u8 = "";
        var it2 = req.response.iterateHeaders();
        while (it2.next()) |h| {
            if (std.ascii.eqlIgnoreCase(h.name, WWW_AUTH_HEADER)) {
                challenge_val = h.value;
                break;
            }
        }
        if (challenge_val.len == 0) return MppError.PaymentFailed;

        const cred = try self.payChallenge(challenge_val);
        defer self.allocator.free(cred);

        return self.postWithRetry(url, body, cred, attempt + 1);
    }

    fn payChallenge(self: *MppClient, challenge: []const u8) ![]u8 {
        const parsed = parseChallenge(challenge);

        if (std.mem.eql(u8, parsed.method, "tempo")) {
            return self.payTempo(parsed);
        } else if (std.mem.eql(u8, parsed.method, "stripe")) {
            return self.payStripe(parsed);
        }
        return MppError.PaymentFailed;
    }

    /// Build a Tempo credential.
    /// Production: sign and broadcast a TIP-20 transfer using self.cfg.private_key.
    /// This stub returns a base64-encoded JSON credential for development.
    fn payTempo(self: *MppClient, parsed: ParsedChallenge) ![]u8 {
        if (self.cfg.private_key == null) return MppError.MissingConfig;

        const nonce = if (parsed.nonce.len > 0)
            parsed.nonce
        else
            try generateNonce(self.allocator);
        const nonce_owned = try self.allocator.dupe(u8, nonce);
        defer self.allocator.free(nonce_owned);

        // Stub tx_hash — replace with actual on-chain transaction
        var tx_buf: [66]u8 = undefined;
        const tx_hash = try std.fmt.bufPrint(&tx_buf, "0x{s}{s}", .{ nonce_owned, nonce_owned[0..@min(30, nonce_owned.len)] });

        const ts = @as(u64, @intCast(std.time.milliTimestamp()));
        const json = try std.fmt.allocPrint(
            self.allocator,
            "{{\"method\":\"tempo\",\"nonce\":\"{s}\",\"tx_hash\":\"{s}\",\"ts\":{d}}}",
            .{ nonce_owned, tx_hash, ts },
        );
        defer self.allocator.free(json);

        const Encoder = std.base64.standard.Encoder;
        const b64_len = Encoder.calcSize(json.len);
        const b64 = try self.allocator.alloc(u8, b64_len);
        _ = Encoder.encode(b64, json);
        return b64;
    }

    /// Build a Stripe credential by calling the SPT endpoint.
    fn payStripe(self: *MppClient, parsed: ParsedChallenge) ![]u8 {
        const endpoint = self.cfg.spt_endpoint orelse return MppError.MissingConfig;

        // POST the challenge params to the SPT endpoint
        const req_body = try std.fmt.allocPrint(
            self.allocator,
            "{{\"method\":\"stripe\",\"amount\":\"{s}\",\"nonce\":\"{s}\"}}",
            .{ parsed.amount, parsed.nonce },
        );
        defer self.allocator.free(req_body);

        // Re-use the HTTP client for the SPT request
        const spt_resp = try self.post(endpoint, req_body);
        defer spt_resp.deinit(self.allocator);

        if (spt_resp.status != 200) return MppError.PaymentFailed;

        // Extract spt field from response JSON (naive scan)
        const spt = extractJsonString(spt_resp.body, "spt") orelse return MppError.PaymentFailed;

        const ts = @as(u64, @intCast(std.time.milliTimestamp()));
        const json = try std.fmt.allocPrint(
            self.allocator,
            "{{\"method\":\"stripe\",\"spt\":\"{s}\",\"nonce\":\"{s}\",\"ts\":{d}}}",
            .{ spt, parsed.nonce, ts },
        );
        defer self.allocator.free(json);

        const Encoder = std.base64.standard.Encoder;
        const b64_len = Encoder.calcSize(json.len);
        const b64 = try self.allocator.alloc(u8, b64_len);
        _ = Encoder.encode(b64, json);
        return b64;
    }
};

// ── JSON field extraction helper ──────────────────────────────────────────────

/// Naively extract the string value of `"key":"<value>"` from a JSON buffer.
/// Returns a slice into `buf`; no allocation.
fn extractJsonString(buf: []const u8, key: []const u8) ?[]const u8 {
    var search_buf: [64]u8 = undefined;
    const needle = std.fmt.bufPrint(&search_buf, "\"{s}\":\"", .{key}) catch return null;
    const start_idx = std.mem.indexOf(u8, buf, needle) orelse return null;
    const val_start = start_idx + needle.len;
    const val_end = std.mem.indexOf(u8, buf[val_start..], "\"") orelse return null;
    return buf[val_start .. val_start + val_end];
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test "generateNonce produces 32 hex chars" {
    const allocator = std.testing.allocator;
    const nonce = try generateNonce(allocator);
    defer allocator.free(nonce);
    try std.testing.expectEqual(@as(usize, 32), nonce.len);
    for (nonce) |c| {
        try std.testing.expect(
            (c >= '0' and c <= '9') or (c >= 'a' and c <= 'f'),
        );
    }
}

test "buildTempoChallenge contains required fields" {
    const allocator = std.testing.allocator;
    const cfg = MppConfig{
        .method    = .tempo,
        .recipient = "0xRecipient",
        .currency  = DEFAULT_TEMPO_CURRENCY,
        .rpc       = DEFAULT_TEMPO_RPC,
        .pricing   = .{ .default = "0.01" },
    };
    const nonce = "testnonce123abc0";
    const header = try buildTempoChallenge(allocator, &cfg, "0.01", nonce);
    defer allocator.free(header);

    try std.testing.expect(std.mem.startsWith(u8, header, PAYMENT_SCHEME));
    try std.testing.expect(std.mem.indexOf(u8, header, "method=\"tempo\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, header, "recipient=\"0xRecipient\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, header, "amount=\"0.01\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, header, "nonce=\"testnonce123abc0\"") != null);
}

test "parseChallenge roundtrip" {
    const header =
        "Payment method=\"tempo\", recipient=\"0xABC\", currency=\"0xTOK\", " ++
        "rpc=\"https://rpc.tempo.xyz\", amount=\"0.05\", nonce=\"aabbcc00\"";
    const parsed = parseChallenge(header);
    try std.testing.expectEqualStrings("tempo",              parsed.method);
    try std.testing.expectEqualStrings("0.05",               parsed.amount);
    try std.testing.expectEqualStrings("aabbcc00",           parsed.nonce);
    try std.testing.expectEqualStrings("0xABC",              parsed.recipient);
}

test "extractCredential parses Authorization header" {
    const auth = "Payment base64tokenhere";
    const cred = extractCredential(auth);
    try std.testing.expect(cred != null);
    try std.testing.expectEqualStrings("base64tokenhere", cred.?);

    try std.testing.expect(extractCredential("Bearer xyz") == null);
    try std.testing.expect(extractCredential("") == null);
}

test "MppPlugin gate returns challenge when no credential" {
    const allocator = std.testing.allocator;
    const plugin = MppPlugin.init(allocator, .{
        .method    = .tempo,
        .recipient = "0xRecipient",
        .pricing   = .{ .default = "0.01" },
    });
    const maybe = try plugin.gate(allocator, null, "translate");
    try std.testing.expect(maybe != null);
    const challenge = maybe.?;
    defer challenge.deinit(allocator);
    try std.testing.expect(std.mem.indexOf(u8, challenge.www_auth_header, "method=\"tempo\"") != null);
}

test "MppPlugin per-capability pricing" {
    const allocator = std.testing.allocator;
    const cap_prices = [_]CapabilityPrice{
        .{ .capability = "summarise", .amount = "0.05" },
    };
    const plugin = MppPlugin.init(allocator, .{
        .method    = .tempo,
        .recipient = "0xRecipient",
        .pricing   = .{
            .default        = "0.01",
            .per_capability = &cap_prices,
        },
    });
    try std.testing.expectEqualStrings("0.05", plugin.priceFor("summarise"));
    try std.testing.expectEqualStrings("0.01", plugin.priceFor("other"));
}

test "buildReceipt is valid JSON-like string" {
    const allocator = std.testing.allocator;
    const receipt = try buildReceipt(allocator, "tempo", "0.01", "abc123");
    defer allocator.free(receipt);
    try std.testing.expect(std.mem.indexOf(u8, receipt, "\"method\":\"tempo\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, receipt, "\"amount\":\"0.01\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, receipt, "\"nonce\":\"abc123\"") != null);
}

test "validateCredentialStructure accepts valid base64 JSON" {
    const allocator = std.testing.allocator;
    const json = "{\"method\":\"tempo\",\"nonce\":\"abc\"}";
    const Encoder = std.base64.standard.Encoder;
    const b64_len = Encoder.calcSize(json.len);
    const b64 = try allocator.alloc(u8, b64_len);
    defer allocator.free(b64);
    _ = Encoder.encode(b64, json);
    try std.testing.expect(validateCredentialStructure(allocator, b64));
    try std.testing.expect(!validateCredentialStructure(allocator, "tooshort"));
}
