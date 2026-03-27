//! Example: **gossip fan-out** for capability discovery (in-process demo).
//!
//! Each [`GossipNode`] keeps a local map `agent_id → capability`. When node **A** announces,
//! the message is pushed to every linked peer with **TTL**; each peer merges into its registry and
//! re-forwards with `ttl - 1` until TTL is exhausted. Duplicates are ignored via a seen set.
//!
//! This mirrors the TypeScript `GossipDiscovery` pattern without real sockets — swap the
//! `deliver` calls for libp2p GossipSub or HTTP fan-out in production.
//!
//! Build: `zig build examples`

const std = @import("std");

pub const GossipMessage = struct {
    kind: []const u8,
    sender_id: []const u8,
    timestamp_ms: i64,
    ttl: u32,
    nonce: u64,
    agent_id: []const u8,
    capability: []const u8,
};

pub const GossipNode = struct {
    allocator: std.mem.Allocator,
    id_buf: []u8,
    id: []const u8,
    default_ttl: u32,
    /// agent_id → capability (demo stores one capability per agent)
    registry: std.StringHashMapUnmanaged([]const u8),
    peers: std.ArrayListUnmanaged(*GossipNode),
    seen: std.AutoHashMapUnmanaged(u128, bool),

    pub fn init(allocator: std.mem.Allocator, id: []const u8, default_ttl: u32) !GossipNode {
        const id_copy = try allocator.dupe(u8, id);
        return .{
            .allocator = allocator,
            .id_buf = id_copy,
            .id = id_copy,
            .default_ttl = default_ttl,
            .registry = .empty,
            .peers = .empty,
            .seen = .empty,
        };
    }

    pub fn deinit(self: *GossipNode) void {
        var it = self.registry.iterator();
        while (it.next()) |kv| {
            self.allocator.free(kv.key_ptr.*);
            self.allocator.free(kv.value_ptr.*);
        }
        self.registry.deinit(self.allocator);
        self.peers.deinit(self.allocator);
        self.seen.deinit(self.allocator);
        self.allocator.free(self.id_buf);
    }

    pub fn linkBidirectional(a: *GossipNode, b: *GossipNode) !void {
        try a.peers.append(a.allocator, b);
        try b.peers.append(b.allocator, a);
    }

    fn seenKey(msg: GossipMessage) u128 {
        const t: u128 = @as(u64, @bitCast(msg.timestamp_ms));
        const n: u128 = msg.nonce;
        const h1: u128 = std.hash.Wyhash.hash(0, msg.sender_id);
        const h2: u128 = std.hash.Wyhash.hash(1, msg.agent_id);
        return t ^ (n << 1) ^ h1 ^ (h2 << 32);
    }

    /// Receive, update registry, fan-out with decremented TTL.
    pub fn deliver(self: *GossipNode, msg: GossipMessage) !void {
        const sk = seenKey(msg);
        if (self.seen.contains(sk)) return;
        try self.seen.put(self.allocator, sk, true);
        errdefer _ = self.seen.remove(sk);

        if (std.mem.eql(u8, msg.kind, "announce")) {
            if (self.registry.fetchRemove(msg.agent_id)) |prev| {
                self.allocator.free(prev.key);
                self.allocator.free(prev.value);
            }
            const owned_agent = try self.allocator.dupe(u8, msg.agent_id);
            errdefer self.allocator.free(owned_agent);
            const owned_cap = try self.allocator.dupe(u8, msg.capability);
            errdefer self.allocator.free(owned_cap);
            try self.registry.put(self.allocator, owned_agent, owned_cap);
        } else if (std.mem.eql(u8, msg.kind, "revoke")) {
            if (self.registry.fetchRemove(msg.agent_id)) |prev| {
                self.allocator.free(prev.key);
                self.allocator.free(prev.value);
            }
        }

        if (msg.ttl == 0) return;

        const next_ttl = msg.ttl - 1;
        const nonce = msg.nonce + 1;

        for (self.peers.items) |peer| {
            if (std.mem.eql(u8, peer.id, msg.sender_id)) continue;
            const fwd = GossipMessage{
                .kind = msg.kind,
                .sender_id = self.id,
                .timestamp_ms = msg.timestamp_ms,
                .ttl = next_ttl,
                .nonce = nonce,
                .agent_id = msg.agent_id,
                .capability = msg.capability,
            };
            try peer.deliver(fwd);
        }
    }

    pub fn announce(self: *GossipNode, agent_id: []const u8, capability: []const u8) !void {
        const msg = GossipMessage{
            .kind = "announce",
            .sender_id = self.id,
            .timestamp_ms = std.time.milliTimestamp(),
            .ttl = self.default_ttl,
            .nonce = @as(u64, @truncate(@as(u64, @bitCast(std.time.milliTimestamp())))),
            .agent_id = agent_id,
            .capability = capability,
        };
        try self.deliver(msg);
    }

    pub fn queryCapability(self: *GossipNode, capability: []const u8, out: *std.ArrayList([]const u8)) !void {
        var it = self.registry.iterator();
        while (it.next()) |kv| {
            if (std.mem.eql(u8, kv.value_ptr.*, capability)) {
                try out.append(kv.key_ptr.*);
            }
        }
    }
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    var a = try GossipNode.init(allocator, "borgkit://agent/a", 3);
    defer a.deinit();
    var b = try GossipNode.init(allocator, "borgkit://agent/b", 3);
    defer b.deinit();

    try GossipNode.linkBidirectional(&a, &b);

    try a.announce("borgkit://agent/service-1", "echo");

    var hits = std.ArrayList([]const u8).init(allocator);
    defer hits.deinit();
    try b.queryCapability("echo", &hits);

    const stdout = std.io.getStdOut().writer();
    try stdout.print("peer b sees {d} agent(s) with capability echo\n", .{hits.items.len});
}
