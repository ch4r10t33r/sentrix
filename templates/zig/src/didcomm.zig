//! DIDComm v2 encrypted messaging for Borgkit agents.
//!
//! Implements authenticated encryption (authcrypt) and anonymous encryption
//! (anoncrypt) between DID-identified agents using X25519 ECDH +
//! ChaCha20-Poly1305 AEAD.
//!
//! All crypto operations use Zig's built-in `std.crypto` — zero external
//! dependencies.
//!
//! # Crypto stack
//!   Key agreement : X25519  (`std.crypto.dh.X25519`)
//!   AEAD          : ChaCha20-Poly1305 (`std.crypto.aead.chacha_poly.ChaCha20Poly1305`)
//!   Encoding      : base64url, base58btc (Bitcoin alphabet, implemented here)
//!   DID method    : `did:key` (multicodec x25519-pub prefix `0xec 0x01`)
//!
//! # Usage example
//! ```zig
//! var alice = try DidcommClient.generate(allocator);
//! defer alice.deinit();
//!
//! var bob = try DidcommClient.generate(allocator);
//! defer bob.deinit();
//!
//! // Alice builds and encrypts an INVOKE for Bob (authcrypt)
//! const encrypted_json = try alice.invoke(
//!     allocator, bob.did, "translate",
//!     \\{"text":"hello"}
//! , false);
//! defer allocator.free(encrypted_json);
//!
//! // Bob decrypts
//! const result = try bob.unpack(allocator, encrypted_json);
//! defer result.deinit(allocator);
//!
//! std.log.info("body: {s}", .{result.message.body_json});
//! std.log.info("sender: {?s}", .{result.sender_did});
//! ```

const std = @import("std");
const crypto = std.crypto;
const mem = std.mem;
const fmt = std.fmt;
const json = std.json;
const base64_std = std.base64;

// ── Crypto primitives ─────────────────────────────────────────────────────────

const X25519 = crypto.dh.X25519;
const ChaCha20Poly1305 = crypto.aead.chacha_poly.ChaCha20Poly1305;

pub const KEY_SIZE   = 32;
pub const NONCE_SIZE = ChaCha20Poly1305.nonce_length; // 12
pub const TAG_SIZE   = ChaCha20Poly1305.tag_length;   // 16

// ── Message type URIs ─────────────────────────────────────────────────────────

pub const MSG_INVOKE   = "https://borgkit.dev/didcomm/1.0/invoke";
pub const MSG_RESPONSE = "https://borgkit.dev/didcomm/1.0/response";
pub const MSG_FORWARD  = "https://borgkit.dev/didcomm/1.0/forward";
pub const MSG_PING     = "https://borgkit.dev/didcomm/1.0/ping";
pub const MSG_PONG     = "https://borgkit.dev/didcomm/1.0/pong";

// ── Types ─────────────────────────────────────────────────────────────────────

/// An X25519 keypair (raw 32-byte scalars).
pub const KeyPair = struct {
    public_key:  [KEY_SIZE]u8,
    private_key: [KEY_SIZE]u8,
};

/// A single attachment item.
pub const Attachment = struct {
    id:          []const u8,
    base64_data: ?[]const u8,
    media_type:  ?[]const u8,
};

/// A decrypted DIDComm v2 message.
///
/// All slice fields are allocated with the arena passed to `unpack`.
pub const DidcommMessage = struct {
    /// UUID v4 message identifier.
    id:           []const u8,
    /// Message type URI, e.g. `MSG_INVOKE`.
    msg_type:     []const u8,
    /// Sender DID, or `null` for anoncrypt.
    from:         ?[]const u8,
    /// Recipient DIDs (slice of owned strings).
    to:           []const []const u8,
    /// Unix timestamp (seconds) when the message was created.
    created_time: i64,
    /// Optional expiry timestamp (Unix seconds).
    expires_time: ?i64,
    /// Raw JSON string of the body object, e.g. `{"capability":"echo","input":{}}`.
    body_json:    []const u8,
};

/// Per-recipient entry in the JWE envelope.
pub const RecipientEntry = struct {
    /// `"did:key:z...#key-1"`
    kid:           []const u8,
    /// Base64url-encoded wrapped content key (nonce || ciphertext || tag).
    encrypted_key: []const u8,
};

/// JWE JSON-serialization envelope for an encrypted DIDComm message.
pub const EncryptedMessage = struct {
    /// Base64url-encoded ChaCha20-Poly1305 ciphertext.
    ciphertext:  []const u8,
    /// Base64url-encoded protected header JSON.
    protected:   []const u8,
    /// Base64url-encoded 12-byte nonce.
    iv:          []const u8,
    /// Base64url-encoded 16-byte Poly1305 tag.
    tag:         []const u8,
    /// Per-recipient entries.
    recipients:  []RecipientEntry,
};

/// Result returned by `unpack`.
pub const UnpackResult = struct {
    message:    DidcommMessage,
    sender_did: ?[]const u8,

    /// Free all heap-allocated strings in this result.
    pub fn deinit(self: UnpackResult, allocator: mem.Allocator) void {
        allocator.free(self.message.id);
        allocator.free(self.message.msg_type);
        if (self.message.from) |f| allocator.free(f);
        for (self.message.to) |s| allocator.free(s);
        allocator.free(self.message.to);
        allocator.free(self.message.body_json);
        if (self.sender_did) |s| allocator.free(s);
    }
};

// ── DidcommClient ─────────────────────────────────────────────────────────────

pub const DidcommClient = struct {
    allocator: mem.Allocator,
    /// `"did:key:z..."` — owned by this struct.
    did:       []const u8,
    key_pair:  KeyPair,

    // ── Constructors ─────────────────────────────────────────────────────────

    /// Generate a fresh X25519 keypair and derive the corresponding `did:key` DID.
    ///
    /// Caller must call `deinit()` to free the allocated DID string.
    pub fn generate(allocator: mem.Allocator) !DidcommClient {
        var private_key: [KEY_SIZE]u8 = undefined;
        crypto.random.bytes(&private_key);
        const public_key = try X25519.recoverPublicKey(private_key);
        const did = try encodeDidKey(allocator, public_key);
        return DidcommClient{
            .allocator = allocator,
            .did       = did,
            .key_pair  = .{ .public_key = public_key, .private_key = private_key },
        };
    }

    /// Free the DID string owned by this client.
    pub fn deinit(self: *DidcommClient) void {
        self.allocator.free(self.did);
    }

    // ── Key resolution ───────────────────────────────────────────────────────

    /// Decode the raw 32-byte X25519 public key from a `did:key` string.
    pub fn resolvePublicKey(did: []const u8) ![KEY_SIZE]u8 {
        return decodeDidKey(did);
    }

    // ── Encryption ───────────────────────────────────────────────────────────

    /// Authcrypt: encrypt `message` for `recipient_dids` with sender authentication.
    ///
    /// Returns an owned JSON string of the `EncryptedMessage` envelope.
    /// Caller must free with `allocator.free(result)`.
    pub fn packAuthcrypt(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        message: DidcommMessage,
        recipient_dids: []const []const u8,
    ) ![]u8 {
        return self.pack(allocator, message, recipient_dids, false);
    }

    /// Anoncrypt: encrypt `message` for `recipient_dids` without revealing sender.
    ///
    /// Returns an owned JSON string of the `EncryptedMessage` envelope.
    /// Caller must free with `allocator.free(result)`.
    pub fn packAnoncrypt(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        message: DidcommMessage,
        recipient_dids: []const []const u8,
    ) ![]u8 {
        return self.pack(allocator, message, recipient_dids, true);
    }

    /// Decrypt an incoming encrypted message JSON.
    ///
    /// Returns an `UnpackResult` whose strings are allocated with `allocator`.
    /// Caller must call `result.deinit(allocator)` when done.
    ///
    /// Returns an error if no recipient entry matches this client's key or if
    /// decryption fails.
    pub fn unpack(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        encrypted_json: []const u8,
    ) !UnpackResult {
        // 1. Parse the outer JWE JSON
        const parsed = try json.parseFromSlice(json.Value, allocator, encrypted_json, .{});
        defer parsed.deinit();
        const root = parsed.value.object;

        const protected_b64  = root.get("protected")  orelse return error.MissingField;
        const ciphertext_b64 = root.get("ciphertext") orelse return error.MissingField;
        const iv_b64         = root.get("iv")         orelse return error.MissingField;
        const tag_b64        = root.get("tag")        orelse return error.MissingField;
        const recipients_val = root.get("recipients") orelse return error.MissingField;

        // 2. Decode + parse the protected header
        const header_bytes = try b64uDecode(allocator, protected_b64.string);
        defer allocator.free(header_bytes);

        const hdr_parsed = try json.parseFromSlice(json.Value, allocator, header_bytes, .{});
        defer hdr_parsed.deinit();
        const hdr = hdr_parsed.value.object;

        const alg_val = hdr.get("alg") orelse return error.MissingAlg;
        const alg     = alg_val.string;
        const is_anon = mem.eql(u8, alg, "ECDH+ChaCha20Poly1305");

        // 3. Recover sender/ephemeral public key
        var sender_pub: [KEY_SIZE]u8 = undefined;
        if (is_anon) {
            const epk_val = hdr.get("epk") orelse return error.MissingEpk;
            const epk_bytes = try b64uDecode(allocator, epk_val.string);
            defer allocator.free(epk_bytes);
            if (epk_bytes.len != KEY_SIZE) return error.InvalidEpkLength;
            @memcpy(&sender_pub, epk_bytes);
        } else {
            const skid_val = hdr.get("skid") orelse return error.MissingSkid;
            sender_pub = try decodeDidKey(skid_val.string);
        }

        // 4. Find our recipient entry
        const my_kid_buf = try fmt.allocPrint(allocator, "{s}#key-1", .{self.did});
        defer allocator.free(my_kid_buf);

        var encrypted_key_b64: []const u8 = "";
        var found = false;
        for (recipients_val.array.items) |recipient| {
            const r = recipient.object;
            const hdr_r = (r.get("header") orelse continue).object;
            const kid_v = hdr_r.get("kid") orelse continue;
            if (mem.eql(u8, kid_v.string, my_kid_buf)) {
                const ek_v = r.get("encrypted_key") orelse continue;
                encrypted_key_b64 = ek_v.string;
                found = true;
                break;
            }
        }
        if (!found) return error.NoMatchingRecipient;

        // 5. ECDH + unwrap content key
        const ek_bytes = try b64uDecode(allocator, encrypted_key_b64);
        defer allocator.free(ek_bytes);
        if (ek_bytes.len < NONCE_SIZE + TAG_SIZE + KEY_SIZE) return error.EncryptedKeyTooShort;

        const ek_nonce = ek_bytes[0..NONCE_SIZE].*;
        const ek_ct_and_tag = ek_bytes[NONCE_SIZE..];

        const shared_secret = try X25519.scalarmult(self.key_pair.private_key, sender_pub);

        var content_key: [KEY_SIZE]u8 = undefined;
        try ChaCha20Poly1305.decrypt(
            &content_key,
            ek_ct_and_tag[0..KEY_SIZE],
            ek_ct_and_tag[KEY_SIZE..][0..TAG_SIZE].*,
            "",
            ek_nonce,
            shared_secret,
        );

        // 6. Decrypt body
        const iv_bytes = try b64uDecode(allocator, iv_b64.string);
        defer allocator.free(iv_bytes);
        if (iv_bytes.len != NONCE_SIZE) return error.InvalidNonceLength;
        const body_nonce = iv_bytes[0..NONCE_SIZE].*;

        const body_ct = try b64uDecode(allocator, ciphertext_b64.string);
        defer allocator.free(body_ct);
        const tag_bytes = try b64uDecode(allocator, tag_b64.string);
        defer allocator.free(tag_bytes);
        if (tag_bytes.len != TAG_SIZE) return error.InvalidTagLength;
        const body_tag = tag_bytes[0..TAG_SIZE].*;

        const plaintext = try allocator.alloc(u8, body_ct.len);
        defer allocator.free(plaintext);
        try ChaCha20Poly1305.decrypt(plaintext, body_ct, body_tag, "", body_nonce, content_key);

        // 7. Parse the plaintext DIDComm message JSON
        return try parseMessage(allocator, plaintext,
            if (is_anon) null else hdr.get("skid"));
    }

    // ── Convenience builders ─────────────────────────────────────────────────

    /// Build and encrypt an INVOKE message to `recipient_did`.
    ///
    /// `body_json` must be a valid JSON object string, e.g. `{"key":"val"}`.
    /// Returns an owned JSON string. Caller must free with `allocator.free(result)`.
    pub fn invoke(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        recipient_did: []const u8,
        capability:    []const u8,
        input_json:    []const u8,
        anon:          bool,
    ) ![]u8 {
        const id  = try generateUuid(allocator);
        defer allocator.free(id);
        const now = std.time.timestamp();
        const from_field: ?[]const u8 = if (anon) null else self.did;
        const body = try fmt.allocPrint(allocator,
            \\{{"capability":"{s}","input":{s}}}
        , .{ capability, input_json });
        defer allocator.free(body);

        const msg = DidcommMessage{
            .id           = id,
            .msg_type     = MSG_INVOKE,
            .from         = from_field,
            .to           = &.{recipient_did},
            .created_time = now,
            .expires_time = null,
            .body_json    = body,
        };

        const dids = [_][]const u8{recipient_did};
        return if (anon)
            self.packAnoncrypt(allocator, msg, &dids)
        else
            self.packAuthcrypt(allocator, msg, &dids);
    }

    /// Build and encrypt a RESPONSE message to `recipient_did`.
    ///
    /// Returns an owned JSON string. Caller must free with `allocator.free(result)`.
    pub fn respond(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        recipient_did: []const u8,
        reply_to_id:   []const u8,
        output_json:   []const u8,
    ) ![]u8 {
        const id  = try generateUuid(allocator);
        defer allocator.free(id);
        const now = std.time.timestamp();
        const body = try fmt.allocPrint(allocator,
            \\{{"reply_to":"{s}","output":{s}}}
        , .{ reply_to_id, output_json });
        defer allocator.free(body);

        const msg = DidcommMessage{
            .id           = id,
            .msg_type     = MSG_RESPONSE,
            .from         = self.did,
            .to           = &.{recipient_did},
            .created_time = now,
            .expires_time = null,
            .body_json    = body,
        };
        const dids = [_][]const u8{recipient_did};
        return self.packAuthcrypt(allocator, msg, &dids);
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Shared encryption implementation for authcrypt and anoncrypt.
    ///
    /// Protocol:
    ///   1. Generate a random 32-byte content key and 12-byte body nonce.
    ///   2. Encrypt the serialized message with ChaCha20-Poly1305.
    ///   3. For each recipient: X25519 ECDH → shared secret →
    ///      ChaCha20-Poly1305 wrap the content key.
    ///   4. Serialize the JWE envelope to JSON.
    fn pack(
        self: *const DidcommClient,
        allocator: mem.Allocator,
        message: DidcommMessage,
        recipient_dids: []const []const u8,
        anon: bool,
    ) ![]u8 {
        // Step 1 — content key + body nonce
        var content_key: [KEY_SIZE]u8 = undefined;
        crypto.random.bytes(&content_key);
        var body_nonce: [NONCE_SIZE]u8 = undefined;
        crypto.random.bytes(&body_nonce);

        // Step 2 — serialize plaintext message
        const plaintext = try serializeMessage(allocator, message);
        defer allocator.free(plaintext);

        // Allocate ciphertext + tag buffers
        var body_ct  = try allocator.alloc(u8, plaintext.len);
        defer allocator.free(body_ct);
        var body_tag: [TAG_SIZE]u8 = undefined;
        ChaCha20Poly1305.encrypt(body_ct, &body_tag, plaintext, "", body_nonce, content_key);

        // Step 3 — sender keypair
        var sender_pub: [KEY_SIZE]u8  = undefined;
        var sender_priv: [KEY_SIZE]u8 = undefined;
        if (anon) {
            crypto.random.bytes(&sender_priv);
            sender_pub = try X25519.recoverPublicKey(sender_priv);
        } else {
            sender_pub  = self.key_pair.public_key;
            sender_priv = self.key_pair.private_key;
        }

        // Build recipient entries
        var recipients_buf = std.ArrayList(u8).init(allocator);
        defer recipients_buf.deinit();
        const rw = recipients_buf.writer();

        try rw.writeAll("[");
        for (recipient_dids, 0..) |did, i| {
            const recipient_pub = try decodeDidKey(did);
            const shared = try X25519.scalarmult(sender_priv, recipient_pub);

            var ek_nonce: [NONCE_SIZE]u8 = undefined;
            crypto.random.bytes(&ek_nonce);
            var ek_ct:  [KEY_SIZE]u8  = undefined;
            var ek_tag: [TAG_SIZE]u8  = undefined;
            ChaCha20Poly1305.encrypt(&ek_ct, &ek_tag, &content_key, "", ek_nonce, shared);

            // Pack as base64url(nonce || ciphertext || tag)
            var ek_raw: [NONCE_SIZE + KEY_SIZE + TAG_SIZE]u8 = undefined;
            @memcpy(ek_raw[0..NONCE_SIZE],                       &ek_nonce);
            @memcpy(ek_raw[NONCE_SIZE..NONCE_SIZE + KEY_SIZE],   &ek_ct);
            @memcpy(ek_raw[NONCE_SIZE + KEY_SIZE..],             &ek_tag);

            const ek_b64  = try b64uEncode(allocator, &ek_raw);
            defer allocator.free(ek_b64);
            const kid_str = try fmt.allocPrint(allocator, "{s}#key-1", .{did});
            defer allocator.free(kid_str);

            if (i > 0) try rw.writeAll(",");
            try rw.print(
                \\{{"header":{{"kid":"{s}"}},"encrypted_key":"{s}"}}
            , .{ kid_str, ek_b64 });
        }
        try rw.writeAll("]");

        // Step 4 — protected header JSON
        var hdr_buf = std.ArrayList(u8).init(allocator);
        defer hdr_buf.deinit();
        const hw = hdr_buf.writer();

        if (anon) {
            const epk_b64 = try b64uEncode(allocator, &sender_pub);
            defer allocator.free(epk_b64);
            try hw.print(
                \\{{"alg":"ECDH+ChaCha20Poly1305","enc":"ChaCha20Poly1305","epk":"{s}"}}
            , .{epk_b64});
        } else {
            try hw.print(
                \\{{"alg":"ECDH-1PU+ChaCha20Poly1305","enc":"ChaCha20Poly1305","skid":"{s}"}}
            , .{self.did});
        }

        const protected_b64  = try b64uEncode(allocator, hdr_buf.items);
        defer allocator.free(protected_b64);
        const ciphertext_b64 = try b64uEncode(allocator, body_ct);
        defer allocator.free(ciphertext_b64);
        const iv_b64         = try b64uEncode(allocator, &body_nonce);
        defer allocator.free(iv_b64);
        const tag_b64        = try b64uEncode(allocator, &body_tag);
        defer allocator.free(tag_b64);

        // Assemble final JSON
        return fmt.allocPrint(allocator,
            \\{{"ciphertext":"{s}","protected":"{s}","recipients":{s},"iv":"{s}","tag":"{s}"}}
        , .{
            ciphertext_b64,
            protected_b64,
            recipients_buf.items,
            iv_b64,
            tag_b64,
        });
    }
};

// ── did:key helpers ───────────────────────────────────────────────────────────

/// X25519 multicodec prefix: `0xec 0x01`.
const MULTICODEC_X25519_PUB = [2]u8{ 0xec, 0x01 };

/// Encode a 32-byte X25519 public key as a `did:key` DID.
///
/// Format: `"did:key:z"` + base58btc(`0xec 0x01` || pubkey)
fn encodeDidKey(allocator: mem.Allocator, pub_key: [KEY_SIZE]u8) ![]u8 {
    var prefixed: [2 + KEY_SIZE]u8 = undefined;
    @memcpy(prefixed[0..2], &MULTICODEC_X25519_PUB);
    @memcpy(prefixed[2..],  &pub_key);
    const b58  = try base58Encode(allocator, &prefixed);
    defer allocator.free(b58);
    return fmt.allocPrint(allocator, "did:key:z{s}", .{b58});
}

/// Decode a `did:key` DID and return the raw 32-byte X25519 public key.
fn decodeDidKey(did: []const u8) ![KEY_SIZE]u8 {
    const prefix = "did:key:z";
    if (!mem.startsWith(u8, did, prefix)) return error.InvalidDidKeyPrefix;
    const encoded = did[prefix.len..];

    // base58Decode needs an allocator; use a small stack allocator
    var buf: [64]u8 = undefined;
    var fba = std.heap.FixedBufferAllocator.init(&buf);
    const decoded = try base58Decode(fba.allocator(), encoded);
    if (decoded.len < 2 + KEY_SIZE) return error.DidKeyPayloadTooShort;
    var key: [KEY_SIZE]u8 = undefined;
    @memcpy(&key, decoded[2..2 + KEY_SIZE]);
    return key;
}

// ── base58btc (Bitcoin alphabet) ──────────────────────────────────────────────

const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Encode bytes to base58btc (no multibase prefix character).
fn base58Encode(allocator: mem.Allocator, data: []const u8) ![]u8 {
    // Convert bytes to a big-endian big integer (stored in a u512 for simplicity
    // with small keys; for a general implementation use an arbitrary-precision type).
    // For 34 bytes (2-byte prefix + 32-byte key) this fits in a u512.
    var n: u512 = 0;
    for (data) |byte| {
        n = n * 256 + @as(u512, byte);
    }

    var out = std.ArrayList(u8).init(allocator);
    errdefer out.deinit();

    if (n == 0) {
        try out.append('1');
    } else {
        while (n > 0) {
            const rem: u9 = @intCast(n % 58);
            n = n / 58;
            try out.append(BASE58_ALPHABET[rem]);
        }
    }
    // Leading zero bytes → leading '1's
    for (data) |byte| {
        if (byte != 0) break;
        try out.append('1');
    }

    // Reverse the result
    const slice = try out.toOwnedSlice();
    mem.reverse(u8, slice);
    return slice;
}

/// Decode a base58btc string (no multibase prefix character) to bytes.
fn base58Decode(allocator: mem.Allocator, encoded: []const u8) ![]u8 {
    var n: u512 = 0;
    for (encoded) |ch| {
        const idx = mem.indexOfScalar(u8, BASE58_ALPHABET, ch) orelse
            return error.InvalidBase58Character;
        n = n * 58 + @as(u512, idx);
    }

    // Count leading '1's → leading zero bytes
    var leading_zeros: usize = 0;
    for (encoded) |ch| {
        if (ch != '1') break;
        leading_zeros += 1;
    }

    // Compute byte length: ceil(log256(n)) + leading_zeros
    var tmp = n;
    var byte_count: usize = 0;
    while (tmp > 0) {
        tmp /= 256;
        byte_count += 1;
    }

    const total = leading_zeros + byte_count;
    const out   = try allocator.alloc(u8, total);
    @memset(out, 0);

    var i: usize = total;
    var val = n;
    while (val > 0) {
        i -= 1;
        out[i] = @intCast(val % 256);
        val /= 256;
    }
    return out;
}

// ── base64url (no padding) ────────────────────────────────────────────────────

const B64URL = base64_std.url_safe_no_pad;

/// Encode bytes to base64url with no padding.
fn b64uEncode(allocator: mem.Allocator, data: []const u8) ![]u8 {
    const len = B64URL.Encoder.calcSize(data.len);
    const out = try allocator.alloc(u8, len);
    _ = B64URL.Encoder.encode(out, data);
    return out;
}

/// Decode a base64url string (with or without padding).
fn b64uDecode(allocator: mem.Allocator, encoded: []const u8) ![]u8 {
    // Strip padding if present
    var stripped = encoded;
    while (stripped.len > 0 and stripped[stripped.len - 1] == '=') {
        stripped = stripped[0..stripped.len - 1];
    }
    const len = try B64URL.Decoder.calcSizeForSlice(stripped);
    const out = try allocator.alloc(u8, len);
    try B64URL.Decoder.decode(out, stripped);
    return out;
}

// ── Message serialization / parsing ──────────────────────────────────────────

/// Serialize a `DidcommMessage` to a JSON byte slice.
/// Caller owns the returned memory.
fn serializeMessage(allocator: mem.Allocator, msg: DidcommMessage) ![]u8 {
    var buf = std.ArrayList(u8).init(allocator);
    errdefer buf.deinit();
    const w = buf.writer();

    try w.writeAll("{");
    try w.print("\"id\":\"{s}\"", .{msg.id});
    try w.print(",\"type\":\"{s}\"", .{msg.msg_type});
    if (msg.from) |f| try w.print(",\"from\":\"{s}\"", .{f});

    try w.writeAll(",\"to\":[");
    for (msg.to, 0..) |did, i| {
        if (i > 0) try w.writeAll(",");
        try w.print("\"{s}\"", .{did});
    }
    try w.writeAll("]");

    try w.print(",\"created_time\":{d}", .{msg.created_time});
    if (msg.expires_time) |exp| try w.print(",\"expires_time\":{d}", .{exp});
    try w.print(",\"body\":{s}", .{msg.body_json});
    try w.writeAll("}");

    return buf.toOwnedSlice();
}

/// Parse raw JSON bytes into a `DidcommMessage`, allocating all strings.
/// Caller must call `result.deinit(allocator)` when done.
fn parseMessage(
    allocator: mem.Allocator,
    data: []const u8,
    skid_val: ?json.Value,
) !UnpackResult {
    const parsed = try json.parseFromSlice(json.Value, allocator, data, .{});
    defer parsed.deinit();
    const obj = parsed.value.object;

    const id_str = (obj.get("id") orelse return error.MissingId).string;
    const type_str = (obj.get("type") orelse return error.MissingType).string;

    const id   = try allocator.dupe(u8, id_str);
    errdefer allocator.free(id);
    const typ  = try allocator.dupe(u8, type_str);
    errdefer allocator.free(typ);

    var from: ?[]u8 = null;
    if (obj.get("from")) |fv| {
        from = try allocator.dupe(u8, fv.string);
    }
    errdefer if (from) |f| allocator.free(f);

    const to_arr = (obj.get("to") orelse return error.MissingTo).array;
    const to_slice = try allocator.alloc([]const u8, to_arr.items.len);
    errdefer allocator.free(to_slice);
    for (to_arr.items, 0..) |item, i| {
        to_slice[i] = try allocator.dupe(u8, item.string);
    }
    errdefer for (to_slice) |s| allocator.free(s);

    const created_time = (obj.get("created_time") orelse return error.MissingCreatedTime).integer;
    const expires_time: ?i64 = if (obj.get("expires_time")) |ev| ev.integer else null;

    // Re-serialize the body back to a JSON string for the caller
    const body_val = obj.get("body") orelse return error.MissingBody;
    var body_buf = std.ArrayList(u8).init(allocator);
    errdefer body_buf.deinit();
    try json.stringify(body_val, .{}, body_buf.writer());
    const body_json = try body_buf.toOwnedSlice();
    errdefer allocator.free(body_json);

    var sender_did: ?[]u8 = null;
    if (skid_val) |sv| {
        sender_did = try allocator.dupe(u8, sv.string);
    }

    return UnpackResult{
        .message = .{
            .id           = id,
            .msg_type     = typ,
            .from         = from,
            .to           = to_slice,
            .created_time = created_time,
            .expires_time = expires_time,
            .body_json    = body_json,
        },
        .sender_did = sender_did,
    };
}

// ── UUID v4 generation ────────────────────────────────────────────────────────

/// Generate a UUID v4 string (crypto-random). Caller owns the memory.
fn generateUuid(allocator: mem.Allocator) ![]u8 {
    var bytes: [16]u8 = undefined;
    crypto.random.bytes(&bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant bits
    return fmt.allocPrint(allocator,
        "{x:0>2}{x:0>2}{x:0>2}{x:0>2}-{x:0>2}{x:0>2}-{x:0>2}{x:0>2}-{x:0>2}{x:0>2}-{x:0>2}{x:0>2}{x:0>2}{x:0>2}{x:0>2}{x:0>2}",
        .{
            bytes[0],  bytes[1],  bytes[2],  bytes[3],
            bytes[4],  bytes[5],
            bytes[6],  bytes[7],
            bytes[8],  bytes[9],
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        },
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test "did:key roundtrip" {
    const allocator = std.testing.allocator;
    var client = try DidcommClient.generate(allocator);
    defer client.deinit();
    try std.testing.expect(mem.startsWith(u8, client.did, "did:key:z"));
    const decoded = try decodeDidKey(client.did);
    const re_did = try encodeDidKey(allocator, decoded);
    defer allocator.free(re_did);
    try std.testing.expectEqualStrings(client.did, re_did);
}

test "authcrypt roundtrip" {
    const allocator = std.testing.allocator;
    var alice = try DidcommClient.generate(allocator);
    defer alice.deinit();
    var bob = try DidcommClient.generate(allocator);
    defer bob.deinit();

    const encrypted = try alice.invoke(
        allocator, bob.did, "echo",
        \\{"msg":"hello borgkit"}
    , false);
    defer allocator.free(encrypted);

    const result = try bob.unpack(allocator, encrypted);
    defer result.deinit(allocator);

    try std.testing.expectEqualStrings(MSG_INVOKE, result.message.msg_type);
    try std.testing.expect(result.sender_did != null);
    try std.testing.expectEqualStrings(alice.did, result.sender_did.?);
}

test "anoncrypt roundtrip" {
    const allocator = std.testing.allocator;
    var alice = try DidcommClient.generate(allocator);
    defer alice.deinit();
    var bob = try DidcommClient.generate(allocator);
    defer bob.deinit();

    const encrypted = try alice.invoke(
        allocator, bob.did, "ping", "{}", true);
    defer allocator.free(encrypted);

    const result = try bob.unpack(allocator, encrypted);
    defer result.deinit(allocator);

    try std.testing.expectEqualStrings(MSG_INVOKE, result.message.msg_type);
    try std.testing.expect(result.sender_did == null); // anoncrypt — no sender
}

test "wrong key fails" {
    const allocator = std.testing.allocator;
    var alice = try DidcommClient.generate(allocator);
    defer alice.deinit();
    var bob = try DidcommClient.generate(allocator);
    defer bob.deinit();
    var carol = try DidcommClient.generate(allocator);
    defer carol.deinit();

    const encrypted = try alice.invoke(
        allocator, bob.did, "secret", "{}", false);
    defer allocator.free(encrypted);

    // Carol tries to decrypt — should fail with NoMatchingRecipient
    const result = carol.unpack(allocator, encrypted);
    try std.testing.expectError(error.NoMatchingRecipient, result);
}
