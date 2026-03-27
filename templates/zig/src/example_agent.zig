//! ExampleAgent — starter template.
//!
//! Demonstrates:
//!   1. Implementing the four required IAgent methods.
//!   2. Overriding getAnr() with a fully-populated DiscoveryEntry.
//!   3. Registering with LocalDiscovery.
//!   4. Using AgentClient to discover and call another agent.
//!
//! Replace the capability implementations with your own logic.
//!
//! ── DIDComm v2 encrypted messaging ───────────────────────────────────────────
//! To send or receive end-to-end encrypted messages between agents, use the
//! `DidcommClient` from `didcomm.zig`. Example:
//!
//!   const didcomm = @import("didcomm.zig");
//!
//!   // One-time setup: generate a persistent did:key keypair for this agent
//!   var my_client = try didcomm.DidcommClient.generate(allocator);
//!   defer my_client.deinit();
//!
//!   // sendEncrypted: invoke a remote agent over an encrypted DIDComm channel
//!   const recipient_did = "did:key:z6Mk..."; // obtain from remote agent's ANR
//!   const encrypted = try my_client.invoke(
//!       allocator, recipient_did, "translate",
//!       \\{"text":"hello"}
//!   , false);
//!   defer allocator.free(encrypted);
//!   // → ship `encrypted` (JSON string) over HTTP/libp2p to the recipient
//!
//!   // receiveEncrypted: decrypt an incoming DIDComm envelope
//!   const result = try my_client.unpack(allocator, encrypted);
//!   defer result.deinit(allocator);
//!   std.log.info("body: {s}", .{result.message.body_json});
//!   // sender_did is null for anoncrypt messages
//!   if (result.sender_did) |sender| std.log.info("from: {s}", .{sender});
//!
//!   // Anonymous send (recipient cannot identify the sender):
//!   const anon_msg = try my_client.invoke(
//!       allocator, recipient_did, "ping", "{}", true);
//!   defer allocator.free(anon_msg);
//!
//!   // Reply to an incoming INVOKE:
//!   if (result.sender_did) |sender| {
//!       const reply = try my_client.respond(
//!           allocator, sender, result.message.id,
//!           \\{"status":"ok"}
//!       );
//!       defer allocator.free(reply);
//!   }

const std = @import("std");
const types = @import("types.zig");
const iface = @import("iagent.zig");
const disc = @import("discovery.zig");
const client_mod = @import("client.zig");

pub const ExampleAgent = struct {
    discovery: disc.LocalDiscovery,

    // ── IAgent interface ───────────────────────────────────────────────────

    pub fn agentId(_: *const ExampleAgent) []const u8 {
        return "borgkit://agent/example";
    }

    pub fn owner(_: *const ExampleAgent) []const u8 {
        return "0xYourWalletAddress";
    }

    pub fn getCapabilities(_: *const ExampleAgent) []const []const u8 {
        return &.{ "echo", "ping" };
    }

    pub fn handleRequest(self: *ExampleAgent, req: types.AgentRequest) types.AgentResponse {
        _ = self;
        if (std.mem.eql(u8, req.capability, "echo")) {
            return types.AgentResponse.success(req.request_id, req.payload);
        } else if (std.mem.eql(u8, req.capability, "ping")) {
            return types.AgentResponse.success(req.request_id,
                \\{"pong":true,"agentId":"borgkit://agent/example","version":"0.1.0"}
            );
        } else {
            return types.AgentResponse.err(req.request_id, "Unknown capability");
        }
    }

    // ── ANR / Identity ─────────────────────────────────────────────────────

    /// Return the fully-populated ANR (Agent Network Record) for this agent.
    ///
    /// Override with your real host, port, and TLS settings before deploying.
    /// The ANR is what other agents use to discover and call you.
    pub fn getAnr(self: *const ExampleAgent) types.DiscoveryEntry {
        return .{
            .agent_id      = self.agentId(),
            .name          = "ExampleAgent",
            .owner         = self.owner(),
            .capabilities  = self.getCapabilities(),
            .network       = .{
                .protocol = .http,
                .host     = "localhost",
                .port     = 6174,
                .tls      = false,
                .peer_id   = "",
                .multiaddr = "",
            },
            .health        = .healthy,
            .registered_at = std.time.milliTimestamp(),
            .metadata_uri  = "ipfs://QmYourMetadataHashHere",
        };
    }

    /// Return the libp2p PeerId for this agent, or null if no signing key is configured.
    ///
    /// To enable: generate a secp256k1 key, persist it between runs, and implement
    /// the secp256k1 → protobuf → SHA2-256 multihash → base58btc derivation.
    /// See src/anr/anr.zig for the derivation helpers.
    pub fn getPeerId(_: *const ExampleAgent) ?[]const u8 {
        return null; // no signing key configured in this example
    }

    // ── Discovery ──────────────────────────────────────────────────────────

    pub fn registerDiscovery(self: *ExampleAgent) !void {
        try self.discovery.register(self.getAnr());
    }

    // ── Compile-time interface validation ─────────────────────────────────
    // This triggers a compile error if the interface contract is broken.
    comptime {
        _ = iface.IAgent(ExampleAgent);
    }
};

// ── Dev runner ────────────────────────────────────────────────────────────────
//
// Demonstrates the full lifecycle:
//   1. Build agent, register with discovery.
//   2. Print ANR and peer ID.
//   3. Local in-process call (no HTTP).
//   4. AgentClient discover-and-call (same discovery, no HTTP in dev mode).
//
pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    // ── 1. Build and register ─────────────────────────────────────────────

    var agent = ExampleAgent{
        .discovery = disc.LocalDiscovery.init(allocator),
    };
    defer agent.discovery.deinit();

    try agent.registerDiscovery();

    // ── 2. Inspect the ANR ───────────────────────────────────────────────
    //
    //  The ANR is the agent's authoritative self-description on the mesh.

    const anr = agent.getAnr();
    std.log.info("Agent ID    : {s}", .{anr.agent_id});
    std.log.info("Owner       : {s}", .{anr.owner});
    std.log.info("Endpoint    : {s}://{s}:{d}", .{
        @tagName(anr.network.protocol),
        anr.network.host,
        anr.network.port,
    });
    {
        var caps_buf = std.ArrayList(u8).init(allocator);
        defer caps_buf.deinit();
        for (anr.capabilities, 0..) |c, i| {
            if (i > 0) try caps_buf.appendSlice(", ");
            try caps_buf.appendSlice(c);
        }
        std.log.info("Capabilities: {s}", .{caps_buf.items});
    }
    if (anr.metadata_uri) |uri| std.log.info("Metadata    : {s}", .{uri});
    if (agent.getPeerId()) |pid| {
        // Build multiaddr from host + port + peerId (libp2p mode):
        // "/ip4/<host>/tcp/<port>/p2p/<peerId>"
        const multiaddr = try std.fmt.allocPrint(
            allocator,
            "/ip4/{s}/tcp/{d}/p2p/{s}",
            .{ anr.network.host, anr.network.port, pid },
        );
        defer allocator.free(multiaddr);
        std.log.info("Peer ID     : {s}", .{pid});
        std.log.info("Multiaddr   : {s}", .{multiaddr});
    } else {
        // Build multiaddr for HTTP mode: "/ip4/<host>/tcp/<port>"
        const multiaddr = try std.fmt.allocPrint(
            allocator,
            "/ip4/{s}/tcp/{d}",
            .{ anr.network.host, anr.network.port },
        );
        defer allocator.free(multiaddr);
        std.log.info("Multiaddr   : {s}", .{multiaddr});
    }

    // ── 3. Local in-process call ──────────────────────────────────────────

    const ping_req = types.AgentRequest{
        .request_id = "test-001",
        .from       = "0xCaller",
        .capability = "ping",
        .payload    = "{}",
    };
    const ping_resp = agent.handleRequest(ping_req);
    std.log.info("[local] ping → status={s} result={s}", .{
        @tagName(ping_resp.status),
        ping_resp.result orelse "null",
    });

    const echo_req = types.AgentRequest{
        .request_id = "test-002",
        .from       = "0xCaller",
        .capability = "echo",
        .payload    = "{\"message\":\"hello borgkit\"}",
    };
    const echo_resp = agent.handleRequest(echo_req);
    std.log.info("[local] echo → status={s} result={s}", .{
        @tagName(echo_resp.status),
        echo_resp.result orelse "null",
    });

    // ── 4. AgentClient — discover-and-call ────────────────────────────────
    //
    //  AgentClient wraps LocalDiscovery and dispatches over HTTP.
    //  In this dev runner the agent is registered in the same LocalDiscovery
    //  instance that the client queries, so find() works without a network hop.
    //
    //  HTTP dispatch (callCapability / call) will fail here because no HTTP
    //  server is running; that is expected in this standalone dev runner.
    //  In production, start an HTTP server and replace LocalDiscovery with
    //  HttpDiscovery or GossipDiscovery.

    var ag_client = client_mod.AgentClient.init(allocator, .{ .local = &agent.discovery }, .{
        .caller_id = "borgkit://agent/caller",
    });
    defer ag_client.deinit();

    // find() — lookup only, no HTTP (free cloned entry when done)
    if (try ag_client.find("ping")) |found| {
        defer disc.freeDiscoveryEntry(allocator, found);
        std.log.info("[client] find(ping) → {s} @ {s}:{d}", .{
            found.agent_id,
            found.network.host,
            found.network.port,
        });
    } else {
        std.log.warn("[client] find(ping) → no agent registered", .{});
    }

    // callCapability() — will attempt HTTP POST to localhost:6174/invoke
    const client_resp = ag_client.callCapability("ping", "{}") catch |err| blk: {
        std.log.warn("[client] callCapability(ping) failed (no HTTP server in dev mode): {}", .{err});
        break :blk types.AgentResponse.err("n/a", "no server");
    };
    std.log.info("[client] callCapability(ping) → status={s}", .{@tagName(client_resp.status)});
}
