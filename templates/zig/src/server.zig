//! Borgkit HTTP server — serves an IAgent-compatible agent over HTTP.
//!
//! Endpoints:
//!   POST /invoke        — dispatch an AgentRequest, return AgentResponse as JSON
//!   POST /invoke/stream — dispatch an AgentRequest, return AgentResponse as SSE stream
//!   POST /gossip        — accept {"from":"...","message":"..."}, log, return {"ok":true}
//!   GET  /health        — return {"status":"healthy","uptime_ms":<elapsed>}
//!   GET  /anr           — return the agent's DiscoveryEntry as JSON
//!   GET  /capabilities  — return {"capabilities":["cap1","cap2",...]}
//!
//! Usage:
//!   try server.serve(MyAgent, &my_agent, 6174, allocator);
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

    std.log.info("[server] Borgkit agent listening on 127.0.0.1:{d}", .{port});

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
    } else if (method == .POST and std.mem.eql(u8, path, "/invoke/stream")) {
        try handleInvokeStream(T, Iface, agent, &req, allocator);
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

// ── POST /invoke/stream ───────────────────────────────────────────────────────
//
// SSE (Server-Sent Events) streaming endpoint.
//
// This handler wraps the single AgentResponse returned by `agent.handleRequest`
// as an SSE stream.  The wire format is:
//
//   data: <AgentResponse JSON>\n\n
//   event: done\n
//   data: {}\n\n
//
// NOTE: True token-by-token streaming (e.g. LLM chunk delivery) requires the
// agent to implement a future `streamRequest` method that yields partial
// results incrementally.  Until that interface is defined, this endpoint
// provides SSE-compatible framing for clients that prefer the event-stream
// content type (e.g. browser EventSource, curl --no-buffer) while keeping the
// same single-round-trip latency as POST /invoke.

fn handleInvokeStream(
    comptime T: type,
    comptime Iface: type,
    agent: *T,
    req: *std.http.Server.Request,
    allocator: std.mem.Allocator,
) !void {
    // ── 1. Read and parse the request body ────────────────────────────────────
    var body_list = std.ArrayList(u8).init(allocator);
    defer body_list.deinit();
    try req.collectBody(&body_list, READ_BUF_SIZE);
    const body = body_list.items;

    const parsed = std.json.parseFromSlice(
        types.AgentRequest,
        allocator,
        body,
        .{ .ignore_unknown_fields = true },
    ) catch |err| {
        std.log.warn("[server] /invoke/stream: JSON parse error: {}", .{err});
        // SSE clients still expect text/event-stream, but we send an error
        // event before closing so the client's onerror handler fires cleanly.
        return sendSseError(req, "invalid JSON body");
    };
    defer parsed.deinit();

    const agent_req = parsed.value;

    // ── 2. x402 payment gating (mirrors /invoke behaviour) ───────────────────
    if (@hasDecl(T, "requiresPayment")) {
        if (agent.requiresPayment()) {
            if (agent_req.payment == null) {
                return sendSseError(req, "payment required");
            }
        }
    }

    // ── 3. Dispatch through the IAgent vtable ─────────────────────────────────
    const response = Iface.dispatch(agent, agent_req);

    // ── 4. Serialize the AgentResponse to JSON ────────────────────────────────
    var json_buf: [RESP_BUF_SIZE]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&json_buf);
    try std.json.stringify(response, .{}, fbs.writer());
    const json_slice = fbs.getWritten();

    // ── 5. Build the full SSE body ────────────────────────────────────────────
    //
    // SSE format (https://html.spec.whatwg.org/multipage/server-sent-events.html):
    //   field: value\n     — named field
    //   \n                 — blank line terminates an event
    //
    // We emit two events:
    //   • An unnamed data event carrying the full AgentResponse JSON.
    //   • A named "done" event with an empty payload so clients can detect EOS.
    //
    // The body is small enough to fit comfortably in RESP_BUF_SIZE.
    var sse_buf: [RESP_BUF_SIZE + 64]u8 = undefined;
    var sse_fbs = std.io.fixedBufferStream(&sse_buf);
    const w = sse_fbs.writer();

    try w.writeAll("data: ");
    try w.writeAll(json_slice);
    try w.writeAll("\n\n");

    try w.writeAll("event: done\n");
    try w.writeAll("data: {}\n\n");

    const sse_body = sse_fbs.getWritten();

    // ── 6. Send the response with SSE headers ─────────────────────────────────
    try req.respond(sse_body, .{
        .status = .ok,
        .extra_headers = &.{
            .{ .name = "Content-Type",  .value = "text/event-stream" },
            .{ .name = "Cache-Control", .value = "no-cache" },
            .{ .name = "Connection",    .value = "keep-alive" },
        },
    });
}

/// Send a minimal SSE response that carries a single error event, allowing
/// clients listening on the event stream to detect failures gracefully.
fn sendSseError(req: *std.http.Server.Request, reason: []const u8) !void {
    var buf: [512]u8 = undefined;
    const body = try std.fmt.bufPrint(
        &buf,
        "event: error\ndata: {{\"error\":\"{s}\"}}\n\nevent: done\ndata: {{}}\n\n",
        .{reason},
    );
    try req.respond(body, .{
        .status = .ok, // SSE connections always open with 200; errors go in events
        .extra_headers = &.{
            .{ .name = "Content-Type",  .value = "text/event-stream" },
            .{ .name = "Cache-Control", .value = "no-cache" },
            .{ .name = "Connection",    .value = "keep-alive" },
        },
    });
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
        \\{{"agent_id":"{s}","name":"{s}","owner":"{s}","capabilities":{s},"network":{{"protocol":"{s}","host":"{s}","port":{d},"tls":{s},"peer_id":"{s}","multiaddr":"{s}"}},"health":"{s}","registered_at":{d},"metadata_uri":{s}}}
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
            entry.network.peer_id,
            entry.network.multiaddr,
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
