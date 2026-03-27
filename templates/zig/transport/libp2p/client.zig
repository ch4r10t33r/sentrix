//! Libp2PAgentClient — Borgkit agent client over libp2p for Zig agents.
//!
//! Dispatches AgentRequests to libp2p peers via the Rust FFI.
//! Falls back to HTTP for entries without a peerId.

const std     = @import("std");
const libp2p  = @import("libp2p.zig");
const json    = std.json;

pub const AgentRequest = struct {
    requestId:  []const u8,
    from:       []const u8,
    capability: []const u8,
    payload:    json.Value,
    timestamp:  u64,
};

pub const AgentResponse = struct {
    requestId:    []const u8,
    status:       []const u8,
    result:       ?json.Value = null,
    errorMessage: ?[]const u8 = null,
    timestamp:    u64,
};

pub const NetworkInfo = struct {
    protocol:  []const u8 = "http",
    host:      []const u8 = "localhost",
    port:      u16 = 6174,
    tls:       bool = false,
    peerId:    ?[]const u8 = null,
    multiaddr: ?[]const u8 = null,
};

pub const DiscoveryEntry = struct {
    agentId:      []const u8,
    capabilities: []const []const u8,
    network:      NetworkInfo,
};

pub const Libp2PAgentClient = struct {
    node:      libp2p.BorgkitNode,
    allocator: std.mem.Allocator,

    pub fn init(node: libp2p.BorgkitNode, allocator: std.mem.Allocator) Libp2PAgentClient {
        return .{ .node = node, .allocator = allocator };
    }

    /// Call a capability on a specific DiscoveryEntry.
    pub fn callEntry(
        self:       *Libp2PAgentClient,
        entry:      DiscoveryEntry,
        capability: []const u8,
        payload:    json.Value,
    ) !AgentResponse {
        if (entry.network.peerId) |peer_id| {
            return self.dispatchP2P(peer_id, capability, payload);
        }
        // HTTP fallback — not implemented in Zig template; requires http client
        return error.HttpFallbackNotImplemented;
    }

    fn dispatchP2P(
        self:       *Libp2PAgentClient,
        peer_id:    []const u8,
        capability: []const u8,
        payload:    json.Value,
    ) !AgentResponse {
        // Build request JSON
        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        const timestamp = @as(u64, @intCast(std.time.milliTimestamp()));
        try std.fmt.format(buf.writer(),
            \\{{"requestId":"{s}","from":"zig-agent","capability":"{s}","timestamp":{d},"payload":
        , .{ randomId(), capability, timestamp });
        try json.stringify(payload, .{}, buf.writer());
        try buf.appendSlice("}");
        try buf.append(0); // null-terminate

        const peer_id_z = try self.allocator.dupeZ(u8, peer_id);
        defer self.allocator.free(peer_id_z);

        const resp_json = try libp2p.send(
            self.node,
            peer_id_z,
            buf.items[0..buf.items.len - 1 :0],
            self.allocator,
        );
        defer self.allocator.free(resp_json);

        const parsed = try json.parseFromSlice(json.Value, self.allocator, resp_json, .{});
        defer parsed.deinit();

        const obj = parsed.value.object;
        return AgentResponse{
            .requestId = obj.get("requestId").?.string,
            .status    = obj.get("status").?.string,
            .timestamp = @intCast(obj.get("timestamp").?.integer),
        };
    }

    fn randomId() [8]u8 {
        var buf: [8]u8 = undefined;
        std.crypto.random.bytes(&buf);
        return buf;
    }
};
