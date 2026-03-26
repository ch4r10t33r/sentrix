//! libp2p.zig — @cImport wrapper for the sentrix-libp2p Rust shared library.
//!
//! Build: link against libsentrix_libp2p.so (or .dylib / .dll)
//! In build.zig:
//!   exe.linkLibC();
//!   exe.addLibraryPath(.{ .path = "path/to/transport/rust/target/release" });
//!   exe.linkSystemLibrary("sentrix_libp2p");

const std = @import("std");

pub const c = @cImport({
    @cInclude("sentrix_libp2p.h");
});

pub const SentrixNode = *c.SentrixHandle;

/// Error type for libp2p operations.
pub const Error = error{
    CreateFailed,
    DialFailed,
    SendFailed,
    GossipFailed,
};

/// Start a new SentrixNode listening on `listen_addr`.
/// Returns an opaque node handle. Caller must call `destroy` when done.
pub fn create(listen_addr: [:0]const u8) Error!SentrixNode {
    const handle = c.sentrix_node_create(listen_addr.ptr, null);
    if (handle == null) return Error.CreateFailed;
    return handle.?;
}

/// Destroy a previously created node.
pub fn destroy(node: SentrixNode) void {
    c.sentrix_node_destroy(node);
}

/// Return the node's PeerId as an allocated string. Caller frees with allocator.
pub fn peerId(node: SentrixNode, allocator: std.mem.Allocator) ![]u8 {
    const raw = c.sentrix_node_peer_id(node);
    defer c.sentrix_free_string(raw);
    return allocator.dupe(u8, std.mem.span(raw));
}

/// Return the node's first listen multiaddr as an allocated string.
pub fn multiaddr(node: SentrixNode, allocator: std.mem.Allocator) ![]u8 {
    const raw = c.sentrix_node_multiaddr(node);
    defer c.sentrix_free_string(raw);
    return allocator.dupe(u8, std.mem.span(raw));
}

/// Dial a remote peer by multiaddr.
pub fn dial(node: SentrixNode, addr: [:0]const u8) Error!void {
    const rc = c.sentrix_dial(node, addr.ptr);
    if (rc != 0) return Error.DialFailed;
}

/// Send a JSON AgentRequest to `peer_id` and return the JSON response.
/// Caller frees the returned slice.
pub fn send(
    node:         SentrixNode,
    peer_id:      [:0]const u8,
    request_json: [:0]const u8,
    allocator:    std.mem.Allocator,
) Error![]u8 {
    const buf_size: usize = 1024 * 1024; // 1 MiB
    const buf = try allocator.alloc(u8, buf_size);
    errdefer allocator.free(buf);

    const n = c.sentrix_send(
        node,
        peer_id.ptr,
        request_json.ptr,
        @ptrCast(buf.ptr),
        buf_size,
    );
    if (n < 0) return Error.SendFailed;

    const result = try allocator.dupe(u8, buf[0..@intCast(n)]);
    allocator.free(buf);
    return result;
}

/// Publish a JSON gossip message to all peers.
pub fn gossipPublish(node: SentrixNode, message_json: [:0]const u8) Error!void {
    const rc = c.sentrix_gossip_publish(node, message_json.ptr);
    if (rc != 0) return Error.GossipFailed;
}
