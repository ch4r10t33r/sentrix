//! Libp2PGossipProtocol — gossip over libp2p GossipSub for Zig agents.

const std    = @import("std");
const libp2p = @import("libp2p.zig");

pub const GossipMessage = struct {
    type:      []const u8,
    senderId:  []const u8,
    timestamp: u64,
    ttl:       u32,
};

pub const Libp2PGossipProtocol = struct {
    node:      libp2p.BorgkitNode,
    allocator: std.mem.Allocator,

    pub fn init(node: libp2p.BorgkitNode, allocator: std.mem.Allocator) Libp2PGossipProtocol {
        return .{ .node = node, .allocator = allocator };
    }

    pub fn broadcast(self: *Libp2PGossipProtocol, msg: GossipMessage) !void {
        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        try std.fmt.format(buf.writer(),
            \\{{"type":"{s}","senderId":"{s}","timestamp":{d},"ttl":{d}}}
        , .{ msg.type, msg.senderId, msg.timestamp, msg.ttl });
        try buf.append(0);
        try libp2p.gossipPublish(self.node, buf.items[0..buf.items.len - 1 :0]);
    }

    pub fn dial(self: *Libp2PGossipProtocol, maddr: []const u8) !void {
        const maddr_z = try self.allocator.dupeZ(u8, maddr);
        defer self.allocator.free(maddr_z);
        try libp2p.dial(self.node, maddr_z);
    }
};
