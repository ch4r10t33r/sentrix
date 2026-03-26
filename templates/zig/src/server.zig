//! Sentrix HTTP server — serves an IAgent-compatible agent over HTTP.
//!
//! Endpoints:
//!   POST /invoke        — dispatch an AgentRequest, return AgentResponse as JSON
//!   POST /gossip        — accept {"from":"...","message":"..."}, log, return {"ok":true}
//!   GET  /health        — return {"status":"healthy","uptime_ms":<elapsed>}
//!   GET  /anr           — return the agent's DiscoveryEntry as JSON
//!   GET  /capabilities  — return {"capabilities":["cap1","cap2",...]}
//!
//! Usage:
//!   try server.serve(MyAgent, &my_agent, 8080, allocator);
//!
//! The server runs a single-threaded accept loop (one connection at a time).
//! x402 payment gating: if T declares `requiresPayment()` and it returns true,
//! requests without a `payment` field receive HTTP 402.

const std = @import("std");
const types = @import("types.zig");
const iagent = @import("iagent.zig");

// ── response buffer size ──────────────────────────────────────────────────────

const RESP_BUF_SIZE: usize = 64 * 1024; // 64 KB stack buffer for JSON responses
const READ_BUF_SIZE: usize = 64 * 1024; // 64 KB for reading request bodies

// ── minimal gossip request shape ─────────────────────────────────────────────

const GossipRequest = struct {
    from: []const u8,
    message: []const u8,
};

// ── serve ─────────────────────────────────────────────────────────────────────

/// Start a single-threaded HTTP server for `agent` on the given `port`.
///
/// Blocks until an unrecoverable error occurs (e.g. bind failure).
/// Each request is processed synchronously before the next connection is accepted.
pub fn serve(comptime T: type, agent: *T, port: u16, allocator: std.mem.Allocator) !void {
    // Compile-time interface validation — triggers a compile error if T does not
    // satisfy the four required IAgent declarations.
    const Iface = iagent.IAgent(T);

    const start_time = std.time.milliTimestamp();

    const addr = try std.net.Address.parseIp4("127.0.0.1", port);
    var http_server = try addr.listen(.{ .reuse_address = true });
    defer http_server.deinit();

    std.log.info("[server] Sentrix agent listening on 127.0.0.1:{d}", .{port});

    // Accept loop — handle one connection at a time.
    while (true) {
        var conn = http_server.accept() catch |err| {
            std.log.err("[server] accept error: {}", .{err});
            continue;
        };
        defer conn.stream.close();

        handleConnection(T, Iface, agent, &conn, start_time, allocator) catch |err| {
            std.log.err("[server] connection handler error: {}", .{err});
        };
    }
}

// ── connection handler ────────────────────────────────────────────────────────

fn handleConnection(
    comptime T: type,
    comptime Iface: type,
    agent: *T,
    conn: *std.net.Server.Connection,
    start_time: i64,
    allocator: std.mem.Allocator,
) !void {
    var read_buffer: [READ_BUF_SIZE]u8 = undefined;
    var http_conn = std.http.Server.init(conn.*, &read_buffer);

    // Process one request per connection (HTTP/1.0 style for simplicity).
    var req = http_conn.receiveHead() catch |err| {
        std.log.warn("[server] receiveHead failed: {}", .{err});
        return;
    };

    const method = req.head.method;
    const target = req.head.target;

    std.log.info("[server] {s} {s}", .{ @tagName(method), target });

    // Strip query string from target for routing
    const path = blk: {
        if (std.mem.indexOf(u8, target, "?")) |q| {
            break :blk target[0..q];
        }
        break :blk target;
    };

    if (method == .POST and std.mem.eql(u8, path, "/invoke")) {
        try handleInvoke(T, Iface, agent, &req, allocator);
    } else if (method == .POST and std.mem.eql(u8, path, "/gossip")) {
        try handleGossip(&req, allocator);
    } else if (method == .GET and std.mem.eql(u8, path, "/health")) {
        try handleHealth(&req, start_time);
    } else if (method == .GET and std.mem.eql(u8, path, "/anr")) {
        try handleAnr(T, Iface, agent, &req, allocator);
    } else if (method == .GET and std.mem.eql(u8, path, "/capabilities")) {
        try handleCapabilities(T, agent, &req);
    } else {
        try sendJson(&req, .not_found,
            \\{"error":"not found","code":"404"}
        );
    }
}

// ── POST /invoke ──────────────────────────────────────────────────────────────

fn handleInvoke(
    comptime T: type,
    comptime Iface: type,
    agent: *T,
    req: *std.http.Server.Request,
    allocator: std.mem.Allocator,
) !void {
    // Read the entire request body.
    var body_list = std.ArrayList(u8).init(allocator);
    defer body_list.deinit();
    try req.collectBody(&body_list, READ_BUF_SIZE);
    const body = body_list.items;

    // Parse the AgentRequest from JSON.
    const parsed = std.json.parseFromSlice(
        types.AgentRequest,
        allocator,
        body,
        .{ .ignore_unknown_fields = true },
    ) catch |err| {
        std.log.warn("[server] /invoke: JSON parse error: {}", .{err});
        return sendJson(req, .bad_request,
            \\{"error":"invalid JSON body","code":"400"}
        );
    };
    defer parsed.deinit();

    const agent_req = parsed.value;

    // x402 payment gating — checked at comptime so there's no runtime overhead
    // for agents that do not declare requiresPayment.
    if (@hasDecl(T, "requiresPayment")) {
        if (agent.requiresPayment()) {
            if (agent_req.payment == null) {
                return sendJson(req, .payment_required,
                    \\{"error":"payment required","code":"402"}
                );
            }
        }
    }

    // Dispatch through the IAgent vtable (runs pre/post hooks if declared).
    const response = Iface.dispatch(agent, agent_req);

    // Serialize the response to JSON.
    var buf: [RESP_BUF_SIZE]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    try std.json.stringify(response, .{}, fbs.writer());
    const json_slice = fbs.getWritten();

    try sendJson(req, .ok, json_slice);
}

// ── POST /gossip ──────────────────────────────────────────────────────────────

fn handleGossip(
    req: *std.http.Server.Request,
    allocator: std.mem.Allocator,
) !void {
    var body_list = std.ArrayList(u8).init(allocator);
    defer body_list.deinit();
    try req.collectBody(&body_list, READ_BUF_SIZE);
    const body = body_list.items;

    const parsed = std.json.parseFromSlice(
        GossipRequest,
        allocator,
        body,
        .{ .ignore_unknown_fields = true },
    ) catch |err| {
        std.log.warn("[server] /gossip: JSON parse error: {}", .{err});
        return sendJson(req, .bad_request,
            \\{"error":"invalid JSON body","code":"400"}
        );
    };
    defer parsed.deinit();

    std.log.info("[gossip] from={s} message={s}", .{
        parsed.value.from,
        parsed.value.message,
    });

    try sendJson(req, .ok, "{\"ok\":true}");
}

// ── GET /health ───────────────────────────────────────────────────────────────

fn handleHealth(req: *std.http.Server.Request, start_time: i64) !void {
    const uptime = std.time.milliTimestamp() - start_time;
    var buf: [256]u8 = undefined;
    const json = try std.fmt.bufPrint(
        &buf,
        "{{\"status\":\"healthy\",\"uptime_ms\":{d}}}",
        .{uptime},
    );
    try sendJson(req, .ok, json);
}

// ── GET /anr ──────────────────────────────────────────────────────────────────

fn handleAnr(
    comptime T: type,
    comptime Iface: type,
    agent: *T,
    req: *std.http.Server.Request,
    allocator: std.mem.Allocator,
) !void {
    const entry = Iface.getAnr(agent, allocator);

    // Build capabilities JSON array manually (slices are not directly
    // serializable with std.json without extra work on enum tags).
    var cap_buf: [4096]u8 = undefined;
    var cap_fbs = std.io.fixedBufferStream(&cap_buf);
    const cap_writer = cap_fbs.writer();
    try cap_writer.writeAll("[");
    for (entry.capabilities, 0..) |cap, i| {
        if (i > 0) try cap_writer.writeAll(",");
        try cap_writer.writeAll("\"");
        try cap_writer.writeAll(cap);
        try cap_writer.writeAll("\"");
    }
    try cap_writer.writeAll("]");
    const caps_json = cap_fbs.getWritten();

    // Build the full DiscoveryEntry JSON manually to avoid issues with
    // enum serialization across Zig versions and optional fields.
    var buf: [RESP_BUF_SIZE]u8 = undefined;
    const json = try std.fmt.bufPrint(&buf,
        \\{{"agent_id":"{s}","name":"{s}","owner":"{s}","capabilities":{s},"network":{{"protocol":"{s}","host":"{s}","port":{d},"tls":{s}}},"health":"{s}","registered_at":{d},"metadata_uri":{s}}}
        ,
        .{
            entry.agent_id,
            entry.name,
            entry.owner,
            caps_json,
            @tagName(entry.network.protocol),
            entry.network.host,
            entry.network.port,
            if (entry.network.tls) "true" else "false",
            @tagName(entry.health),
            entry.registered_at,
            if (entry.metadata_uri) |uri| blk: {
                // Inline quoted string for the optional metadata_uri field.
                // We re-use a small portion of buf after the main slice.
                // Safe: bufPrint already advanced up to `json.len` bytes.
                var tmp: [1024]u8 = undefined;
                const s = std.fmt.bufPrint(&tmp, "\"{s}\"", .{uri}) catch "null";
                break :blk s;
            } else "null",
        },
    );

    try sendJson(req, .ok, json);
}

// ── GET /capabilities ─────────────────────────────────────────────────────────

fn handleCapabilities(comptime T: type, agent: *T, req: *std.http.Server.Request) !void {
    const caps = agent.getCapabilities();

    var buf: [RESP_BUF_SIZE]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    const writer = fbs.writer();

    try writer.writeAll("{\"capabilities\":[");
    for (caps, 0..) |cap, i| {
        if (i > 0) try writer.writeAll(",");
        try writer.writeAll("\"");
        try writer.writeAll(cap);
        try writer.writeAll("\"");
    }
    try writer.writeAll("]}");

    try sendJson(req, .ok, fbs.getWritten());
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Write a complete HTTP response with Content-Type: application/json.
fn sendJson(
    req: *std.http.Server.Request,
    status: std.http.Status,
    body: []const u8,
) !void {
    try req.respond(body, .{
        .status = status,
        .extra_headers = &.{
            .{ .name = "Content-Type", .value = "application/json" },
        },
    });
}
