//! MCP Server — Outbound bridge: Borgkit agent → MCP server
//!
//! Exposes any IAgent-compatible Borgkit agent as a Model Context Protocol
//! server that MCP clients (e.g. Claude Desktop, Cursor, Continue) can connect
//! to.  Supports two transport modes:
//!
//!   • stdio  — reads newline-delimited JSON-RPC from stdin, writes to stdout
//!              (the transport required by Claude Desktop and most MCP clients)
//!   • http   — listens on a TCP port and handles POST /mcp requests
//!
//! There is no official Zig MCP SDK; this file implements the MCP JSON-RPC 2.0
//! wire protocol directly.
//!
//! ── Usage ─────────────────────────────────────────────────────────────────────
//!
//!   const mcp_server = @import("mcp_server.zig");
//!
//!   var agent = MyAgent{ ... };
//!
//!   // Stdio (for Claude Desktop):
//!   try mcp_server.serveAsMcp(MyAgent, &agent, .{}, allocator);
//!
//!   // HTTP:
//!   try mcp_server.serveAsMcp(MyAgent, &agent, .{
//!       .transport = .http,
//!       .port      = 3000,
//!   }, allocator);
//!
//! ── Claude Desktop config ─────────────────────────────────────────────────────
//!
//!   {
//!     "mcpServers": {
//!       "my-borgkit-agent": {
//!         "command": "/path/to/my-agent",
//!         "args": []
//!       }
//!     }
//!   }

const std = @import("std");
const types = @import("types.zig");
const iagent = @import("iagent.zig");

// ── constants ─────────────────────────────────────────────────────────────────

const MAX_LINE: usize    = 4 * 1024 * 1024; // 4 MB per JSON-RPC line (stdio)
const READ_BUF: usize    = 64 * 1024;       // 64 KB HTTP read buffer
const RESP_BUF: usize    = 256 * 1024;      // 256 KB per-request response buffer
const PROTOCOL_VERSION: []const u8 = "2024-11-05";

// ── public types ──────────────────────────────────────────────────────────────

pub const Transport = enum { stdio, http };

pub const ServeMcpOptions = struct {
    /// Server name reported in the MCP initialize handshake.
    /// Defaults to "borgkit-agent".
    name: ?[]const u8 = null,
    transport: Transport = .stdio,
    /// Bind address for HTTP transport.
    host: []const u8 = "0.0.0.0",
    /// Bind port for HTTP transport.
    port: u16 = 3000,
};

// ── serveAsMcp ────────────────────────────────────────────────────────────────

/// Expose a Borgkit agent as an MCP server.
///
/// This is a comptime function: the agent type is checked at compile time for
/// the four required IAgent declarations (agentId, owner, getCapabilities,
/// handleRequest).
///
/// Blocks until EOF (stdio transport) or an unrecoverable accept error (HTTP).
pub fn serveAsMcp(
    comptime AgentType: type,
    agent: *AgentType,
    options: ServeMcpOptions,
    allocator: std.mem.Allocator,
) !void {
    // Compile-time interface validation (mirrors server.zig pattern).
    _ = iagent.IAgent(AgentType);

    const name = options.name orelse "borgkit-agent";

    switch (options.transport) {
        .stdio => try serveStdio(AgentType, agent, name, allocator),
        .http  => try serveHttp(AgentType, agent, name, options, allocator),
    }
}

// ── stdio handler ─────────────────────────────────────────────────────────────

fn serveStdio(
    comptime AgentType: type,
    agent: *AgentType,
    name: []const u8,
    allocator: std.mem.Allocator,
) !void {
    const stdin  = std.io.getStdIn();
    const stdout = std.io.getStdOut();

    std.log.info("[mcp_server] stdio transport ready — name={s}", .{name});

    while (true) {
        // Read one newline-delimited JSON-RPC message.
        const line = stdin.reader().readUntilDelimiterAlloc(
            allocator, '\n', MAX_LINE,
        ) catch |err| switch (err) {
            error.EndOfStream => {
                std.log.info("[mcp_server] stdin EOF — shutting down", .{});
                return;
            },
            else => {
                std.log.err("[mcp_server] stdin read error: {}", .{err});
                return err;
            },
        };
        defer allocator.free(line);

        const trimmed = std.mem.trim(u8, line, " \t\r");
        if (trimmed.len == 0) continue;

        // Per-request arena keeps dispatch memory bounded.
        var arena = std.heap.ArenaAllocator.init(allocator);
        defer arena.deinit();

        const response = dispatchRequest(
            AgentType, agent, name, trimmed, arena.allocator(),
        ) catch |err| blk: {
            std.log.err("[mcp_server] dispatch error: {}", .{err});
            break :blk null;
        };

        if (response) |resp| {
            defer arena.allocator().free(resp);
            // Write response + newline.
            try stdout.writeAll(resp);
            try stdout.writeAll("\n");
        }
    }
}

// ── HTTP handler ──────────────────────────────────────────────────────────────

fn serveHttp(
    comptime AgentType: type,
    agent: *AgentType,
    name: []const u8,
    options: ServeMcpOptions,
    allocator: std.mem.Allocator,
) !void {
    const addr = try std.net.Address.resolveIp(options.host, options.port);
    var listener = try addr.listen(.{ .reuse_address = true });
    defer listener.deinit();

    std.log.info("[mcp_server] HTTP transport listening on {s}:{d}", .{ options.host, options.port });

    while (true) {
        var conn = listener.accept() catch |err| {
            std.log.err("[mcp_server] accept error: {}", .{err});
            continue;
        };
        defer conn.stream.close();

        handleHttpConnection(AgentType, agent, name, &conn, allocator) catch |err| {
            std.log.err("[mcp_server] HTTP connection error: {}", .{err});
        };
    }
}

fn handleHttpConnection(
    comptime AgentType: type,
    agent: *AgentType,
    name: []const u8,
    conn: *std.net.Server.Connection,
    allocator: std.mem.Allocator,
) !void {
    var read_buf: [READ_BUF]u8 = undefined;
    var http_conn = std.http.Server.init(conn.*, &read_buf);

    var req = http_conn.receiveHead() catch |err| {
        std.log.warn("[mcp_server] receiveHead: {}", .{err});
        return;
    };

    const method = req.head.method;
    const target = req.head.target;

    // Strip query string for routing.
    const path = if (std.mem.indexOf(u8, target, "?")) |q| target[0..q] else target;

    if (method == .GET and std.mem.eql(u8, path, "/health")) {
        return sendHttpJson(&req, .ok, "{\"ok\":true}");
    }

    if (method == .POST and std.mem.eql(u8, path, "/mcp")) {
        // Per-request arena.
        var arena = std.heap.ArenaAllocator.init(allocator);
        defer arena.deinit();
        const aa = arena.allocator();

        var body_list = std.ArrayList(u8).init(aa);
        try req.collectBody(&body_list, READ_BUF);
        const body = std.mem.trim(u8, body_list.items, " \t\r\n");

        const response = dispatchRequest(AgentType, agent, name, body, aa) catch |err| blk: {
            std.log.err("[mcp_server] dispatch error: {}", .{err});
            break :blk try jsonRpcError(null, -32603, "Internal error", aa);
        };

        const resp_body = response orelse try jsonRpcError(null, -32600, "Parse error", aa);
        return sendHttpJson(&req, .ok, resp_body);
    }

    try sendHttpJson(&req, .not_found, "{\"error\":\"not found\"}");
}

// ── JSON-RPC dispatcher ───────────────────────────────────────────────────────

/// Parse one JSON-RPC message and return the serialized response (or null for
/// notifications that require no reply).
///
/// All allocations use `allocator`; caller owns the returned slice.
fn dispatchRequest(
    comptime AgentType: type,
    agent: *AgentType,
    server_name: []const u8,
    body: []const u8,
    allocator: std.mem.Allocator,
) !?[]const u8 {
    const parsed = std.json.parseFromSlice(
        std.json.Value, allocator, body, .{ .ignore_unknown_fields = true },
    ) catch {
        return try jsonRpcError(null, -32700, "Parse error", allocator);
    };

    const obj = switch (parsed.value) {
        .object => |o| o,
        else    => return try jsonRpcError(null, -32600, "Invalid Request", allocator),
    };

    const id = extractId(parsed.value);

    // Notifications have no "id" field and expect no response.
    const has_id = obj.contains("id");

    const method_val = obj.get("method") orelse {
        return try jsonRpcError(id, -32600, "Missing method", allocator);
    };
    const method = switch (method_val) {
        .string => |s| s,
        else    => return try jsonRpcError(id, -32600, "Method must be a string", allocator),
    };

    // ── initialize ────────────────────────────────────────────────────────────
    if (std.mem.eql(u8, method, "initialize")) {
        const result = try std.fmt.allocPrint(
            allocator,
            "{{\"protocolVersion\":\"{s}\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"{s}\",\"version\":\"0.1.0\"}}}}",
            .{ PROTOCOL_VERSION, server_name },
        );
        return try jsonRpcSuccess(id, result, allocator);
    }

    // ── notifications/initialized — no response ───────────────────────────────
    if (std.mem.eql(u8, method, "notifications/initialized")) {
        _ = has_id;
        return null;
    }

    // ── tools/list ────────────────────────────────────────────────────────────
    if (std.mem.eql(u8, method, "tools/list")) {
        const caps = agent.getCapabilities();
        const tools_json = try buildToolsList(caps, allocator);
        const result = try std.fmt.allocPrint(allocator, "{{\"tools\":{s}}}", .{tools_json});
        return try jsonRpcSuccess(id, result, allocator);
    }

    // ── tools/call ────────────────────────────────────────────────────────────
    if (std.mem.eql(u8, method, "tools/call")) {
        return try handleToolsCall(AgentType, agent, parsed.value, id, allocator);
    }

    // ── unknown method ────────────────────────────────────────────────────────
    return try jsonRpcError(id, -32601, "Method not found", allocator);
}

fn handleToolsCall(
    comptime AgentType: type,
    agent: *AgentType,
    root: std.json.Value,
    id: ?i64,
    allocator: std.mem.Allocator,
) ![]const u8 {
    const params = switch (root) {
        .object => |o| o.get("params") orelse {
            return jsonRpcError(id, -32602, "Missing params", allocator);
        },
        else => return jsonRpcError(id, -32602, "Invalid params", allocator),
    };

    const params_obj = switch (params) {
        .object => |o| o,
        else    => return jsonRpcError(id, -32602, "params must be object", allocator),
    };

    const tool_name = switch (params_obj.get("name") orelse {
        return jsonRpcError(id, -32602, "Missing params.name", allocator);
    }) {
        .string => |s| s,
        else    => return jsonRpcError(id, -32602, "params.name must be string", allocator),
    };

    // Serialize arguments back to JSON for the AgentRequest payload.
    const payload: []const u8 = blk: {
        const args_val = params_obj.get("arguments") orelse break :blk "{}";
        var buf = std.ArrayList(u8).init(allocator);
        try std.json.stringify(args_val, .{}, buf.writer());
        break :blk try buf.toOwnedSlice();
    };

    // Build a request_id from the JSON-RPC id.
    const req_id: []const u8 = if (id) |n|
        try std.fmt.allocPrint(allocator, "mcp-{d}", .{n})
    else
        "mcp-notif";

    const agent_req = types.AgentRequest{
        .request_id = req_id,
        .from       = "mcp-client",
        .capability = tool_name,
        .payload    = payload,
        .timestamp  = std.time.milliTimestamp(),
    };

    const agent_resp = agent.handleRequest(agent_req);

    // Format as MCP content array.
    const text: []const u8 = if (agent_resp.status == .success)
        agent_resp.result orelse ""
    else
        agent_resp.error_message orelse "error";

    var text_json = std.ArrayList(u8).init(allocator);
    try std.json.encodeJsonString(text, .{}, text_json.writer());

    const result = try std.fmt.allocPrint(
        allocator,
        "{{\"content\":[{{\"type\":\"text\",\"text\":{s}}}]}}",
        .{text_json.items},
    );
    return jsonRpcSuccess(id, result, allocator);
}

// ── response builders ─────────────────────────────────────────────────────────

/// Build {"jsonrpc":"2.0","id":N,"result":RESULT_JSON}
///
/// If `id` is null the id field is omitted.
fn jsonRpcSuccess(
    id: ?i64,
    result_json: []const u8,
    allocator: std.mem.Allocator,
) ![]const u8 {
    if (id) |n| {
        return std.fmt.allocPrint(
            allocator,
            "{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":{s}}}",
            .{ n, result_json },
        );
    }
    return std.fmt.allocPrint(
        allocator,
        "{{\"jsonrpc\":\"2.0\",\"id\":null,\"result\":{s}}}",
        .{result_json},
    );
}

/// Build {"jsonrpc":"2.0","id":N,"error":{"code":CODE,"message":"MSG"}}
fn jsonRpcError(
    id: ?i64,
    code: i32,
    message: []const u8,
    allocator: std.mem.Allocator,
) ![]const u8 {
    var msg_json = std.ArrayList(u8).init(allocator);
    defer msg_json.deinit();
    try std.json.encodeJsonString(message, .{}, msg_json.writer());

    if (id) |n| {
        return std.fmt.allocPrint(
            allocator,
            "{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":{d},\"message\":{s}}}}}",
            .{ n, code, msg_json.items },
        );
    }
    return std.fmt.allocPrint(
        allocator,
        "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":{d},\"message\":{s}}}}}",
        .{ code, msg_json.items },
    );
}

/// Build the MCP tools JSON array from a slice of capability name strings.
///
/// Each capability becomes:
///   {"name":"cap","description":"Borgkit capability: cap",
///    "inputSchema":{"type":"object","properties":{"payload":{"type":"object"}}}}
fn buildToolsList(caps: []const []const u8, allocator: std.mem.Allocator) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    const w = buf.writer();

    try w.writeAll("[");
    for (caps, 0..) |cap, i| {
        if (i > 0) try w.writeAll(",");

        // JSON-encode the capability name for safety.
        var name_json = std.ArrayList(u8).init(allocator);
        defer name_json.deinit();
        try std.json.encodeJsonString(cap, .{}, name_json.writer());

        var desc_raw = std.ArrayList(u8).init(allocator);
        defer desc_raw.deinit();
        try desc_raw.writer().print("Borgkit capability: {s}", .{cap});

        var desc_json = std.ArrayList(u8).init(allocator);
        defer desc_json.deinit();
        try std.json.encodeJsonString(desc_raw.items, .{}, desc_json.writer());

        try w.print(
            "{{\"name\":{s},\"description\":{s}," ++
            "\"inputSchema\":{{\"type\":\"object\",\"properties\":{{\"payload\":{{\"type\":\"object\"}}}}}}}}",
            .{ name_json.items, desc_json.items },
        );
    }
    try w.writeAll("]");

    return buf.toOwnedSlice();
}

/// Extract the "id" field from a JSON-RPC request value.
///
/// Returns null for notifications (no id field) or when the id is not an integer.
fn extractId(root: std.json.Value) ?i64 {
    const obj = switch (root) {
        .object => |o| o,
        else    => return null,
    };
    const id_val = obj.get("id") orelse return null;
    return switch (id_val) {
        .integer => |n| @intCast(n),
        .float   => |f| @intFromFloat(f),
        else     => null,
    };
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn sendHttpJson(
    req: *std.http.Server.Request,
    status: std.http.Status,
    body: []const u8,
) !void {
    try req.respond(body, .{
        .status       = status,
        .extra_headers = &.{
            .{ .name = "Content-Type", .value = "application/json" },
        },
    });
}
