//! MCP Plugin — Inbound bridge: MCP server → Borgkit agent
//!
//! Wraps any MCP-compatible server and exposes its tools as a Borgkit agent.
//! Supports two transport modes:
//!
//!   • stdio  — spawn a subprocess and communicate over its stdin/stdout
//!   • http   — speak JSON-RPC 2.0 over HTTP POST to a running server
//!
//! There is no official Zig MCP SDK; this file implements the MCP JSON-RPC 2.0
//! wire protocol directly as specified at https://spec.modelcontextprotocol.io/.
//!
//! ── Usage (stdio) ─────────────────────────────────────────────────────────────
//!
//!   const mcp = @import("mcp_plugin.zig");
//!
//!   var plugin = mcp.McpPlugin.initStdio(allocator);
//!   defer plugin.deinit();
//!
//!   try plugin.fromCommand(
//!       &.{ "npx", "-y", "@modelcontextprotocol/server-filesystem", "/tmp" },
//!       null,
//!   );
//!
//!   const caps = try plugin.getCapabilities(allocator);
//!   defer { for (caps) |c| allocator.free(c); allocator.free(caps); }
//!
//!   const resp = try plugin.handleRequest(.{
//!       .request_id = "r1",
//!       .from       = "borgkit://me",
//!       .capability = "read_file",
//!       .payload    = "{\"path\":\"/tmp/hello.txt\"}",
//!   }, allocator);
//!
//! ── Usage (HTTP) ──────────────────────────────────────────────────────────────
//!
//!   var plugin = mcp.McpPlugin.initHttp(allocator, .{
//!       .url = "http://localhost:3000/mcp",
//!   });
//!   defer plugin.deinit();
//!   try plugin.fromUrl();
//!
//!   const resp = try plugin.handleRequest(req, allocator);

const std = @import("std");
const types = @import("types.zig");

// ── constants ─────────────────────────────────────────────────────────────────

/// Maximum bytes read per JSON-RPC line (stdio transport).
const MAX_LINE: usize = 4 * 1024 * 1024;

// ── public types ──────────────────────────────────────────────────────────────

/// A single MCP tool descriptor fetched from the remote server.
pub const McpTool = struct {
    name: []const u8,
    description: []const u8,
    /// Raw JSON schema string, or null when the server omits inputSchema.
    input_schema_json: ?[]const u8 = null,
};

/// Transport mode for the MCP connection.
pub const TransportMode = enum { stdio, http };

/// HTTP connection configuration.
pub const HttpConfig = struct {
    /// Full URL of the MCP endpoint, e.g. "http://localhost:3000/mcp".
    url: []const u8,
    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,
};

// ── McpPlugin ─────────────────────────────────────────────────────────────────

pub const McpPlugin = struct {
    allocator: std.mem.Allocator,
    tools: std.ArrayList(McpTool),
    mode: TransportMode,

    // Stdio transport state
    child: ?std.process.Child = null,

    // HTTP transport state
    http_client: ?std.http.Client = null,
    http_config: ?HttpConfig = null,

    /// Monotonically increasing JSON-RPC request id.
    next_id: u64 = 1,

    // ── constructors ─────────────────────────────────────────────────────────

    /// Create a plugin that will communicate with a subprocess via stdio.
    pub fn initStdio(allocator: std.mem.Allocator) McpPlugin {
        return .{
            .allocator = allocator,
            .tools     = std.ArrayList(McpTool).init(allocator),
            .mode      = .stdio,
        };
    }

    /// Create a plugin that will communicate with an HTTP MCP server.
    pub fn initHttp(allocator: std.mem.Allocator, config: HttpConfig) McpPlugin {
        return .{
            .allocator   = allocator,
            .tools       = std.ArrayList(McpTool).init(allocator),
            .mode        = .http,
            .http_config = config,
        };
    }

    // ── deinit ───────────────────────────────────────────────────────────────

    pub fn deinit(self: *McpPlugin) void {
        for (self.tools.items) |tool| {
            self.allocator.free(tool.name);
            self.allocator.free(tool.description);
            if (tool.input_schema_json) |s| self.allocator.free(s);
        }
        self.tools.deinit();

        if (self.child) |*ch| {
            if (ch.stdin) |*s| s.close();
            _ = ch.wait() catch {};
        }

        if (self.http_client) |*c| c.deinit();
    }

    // ── factory: stdio subprocess ─────────────────────────────────────────────

    /// Launch a subprocess MCP server and connect to it via stdio.
    ///
    /// Performs the MCP initialize handshake (initialize + notifications/initialized)
    /// then calls refreshTools() to populate the tool list.
    ///
    /// `argv`      — command + arguments, e.g. &.{"npx", "-y", "server-name"}
    /// `extra_env` — optional key/value pairs merged into the child environment;
    ///               pass null to inherit the parent environment unchanged.
    pub fn fromCommand(
        self: *McpPlugin,
        argv: []const []const u8,
        extra_env: ?[]const struct { key: []const u8, value: []const u8 },
    ) !void {
        var child = std.process.Child.init(argv, self.allocator);
        child.stdin_behavior  = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Inherit;

        if (extra_env) |pairs| {
            var env_map = try std.process.getEnvMap(self.allocator);
            defer env_map.deinit();
            for (pairs) |p| try env_map.put(p.key, p.value);
            child.env_map = &env_map;
        }

        try child.spawn();
        self.child = child;

        // ── initialize request ────────────────────────────────────────────────
        const init_req = try self.buildRequest(
            "initialize",
            \\{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"borgkit","version":"1.0.0"}}
            ,
            self.allocator,
        );
        defer self.allocator.free(init_req);

        const stdin  = self.child.?.stdin  orelse return error.NoPipe;
        const stdout = self.child.?.stdout orelse return error.NoPipe;

        try stdin.writeAll(init_req);

        // Read and discard the initialize response (we just need it to arrive
        // before sending the notification).
        const init_resp = try stdout.reader().readUntilDelimiterAlloc(
            self.allocator, '\n', MAX_LINE,
        );
        defer self.allocator.free(init_resp);
        std.log.debug("[mcp_plugin] initialize: {s}", .{init_resp});

        // ── notifications/initialized (no response expected) ─────────────────
        try stdin.writeAll("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n");

        // ── fetch tools ───────────────────────────────────────────────────────
        try self.refreshTools();
    }

    // ── factory: HTTP transport ───────────────────────────────────────────────

    /// Connect to a running HTTP MCP server, complete the initialize handshake,
    /// then fetch the tool list.
    pub fn fromUrl(self: *McpPlugin) !void {
        self.http_client = std.http.Client{ .allocator = self.allocator };

        // ── initialize handshake ─────────────────────────────────────────────
        const init_req = try self.buildRequest(
            "initialize",
            \\{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"borgkit","version":"1.0.0"}}
            ,
            self.allocator,
        );
        defer self.allocator.free(init_req);

        const init_resp = try self.sendHttpRequest(init_req, self.allocator);
        defer self.allocator.free(init_resp);
        std.log.debug("[mcp_plugin] HTTP initialize: {s}", .{init_resp});

        // ── notifications/initialized (best-effort) ───────────────────────────
        const notif_req = try self.buildRequest(
            "notifications/initialized", "{}", self.allocator,
        );
        defer self.allocator.free(notif_req);
        const notif_resp = self.sendHttpRequest(notif_req, self.allocator) catch |e| blk: {
            std.log.debug("[mcp_plugin] notifications/initialized ignored: {}", .{e});
            break :blk try self.allocator.dupe(u8, "");
        };
        defer self.allocator.free(notif_resp);

        try self.refreshTools();
    }

    // ── tool fetching ─────────────────────────────────────────────────────────

    /// Send tools/list and repopulate self.tools from the response.
    ///
    /// Uses an arena for JSON parsing; string data is copied into self.allocator
    /// so it outlives the arena.
    pub fn refreshTools(self: *McpPlugin) !void {
        const list_req = try self.buildRequest("tools/list", "{}", self.allocator);
        defer self.allocator.free(list_req);

        const raw = switch (self.mode) {
            .stdio => try self.sendStdioRequest(list_req, self.allocator),
            .http  => try self.sendHttpRequest(list_req, self.allocator),
        };
        defer self.allocator.free(raw);

        // Free old tool data.
        for (self.tools.items) |t| {
            self.allocator.free(t.name);
            self.allocator.free(t.description);
            if (t.input_schema_json) |s| self.allocator.free(s);
        }
        self.tools.clearRetainingCapacity();

        // Parse inside a temporary arena.
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();

        const parsed = try std.json.parseFromSlice(
            std.json.Value, arena.allocator(), raw, .{ .ignore_unknown_fields = true },
        );

        // result.tools[]
        const result_val = switch (parsed.value) {
            .object => |obj| obj.get("result") orelse return,
            else    => return,
        };
        const tools_val = switch (result_val) {
            .object => |obj| obj.get("tools") orelse return,
            else    => return,
        };
        const tools_arr = switch (tools_val) {
            .array => |a| a.items,
            else   => return,
        };

        for (tools_arr) |tool_val| {
            const obj = switch (tool_val) {
                .object => |o| o,
                else    => continue,
            };

            const name_raw = switch (obj.get("name") orelse continue) {
                .string => |s| s,
                else    => continue,
            };
            const desc_raw: []const u8 = switch (obj.get("description") orelse .null) {
                .string => |s| s,
                else    => "",
            };

            const name = try self.allocator.dupe(u8, name_raw);
            errdefer self.allocator.free(name);
            const desc = try self.allocator.dupe(u8, desc_raw);
            errdefer self.allocator.free(desc);

            // Optionally persist the inputSchema as a JSON string.
            const schema: ?[]const u8 = blk: {
                const sv = obj.get("inputSchema") orelse break :blk null;
                var buf = std.ArrayList(u8).init(self.allocator);
                errdefer buf.deinit();
                try std.json.stringify(sv, .{}, buf.writer());
                break :blk try buf.toOwnedSlice();
            };
            errdefer if (schema) |s| self.allocator.free(s);

            try self.tools.append(.{
                .name              = name,
                .description       = desc,
                .input_schema_json = schema,
            });
        }

        std.log.info("[mcp_plugin] loaded {d} MCP tool(s)", .{self.tools.items.len});
    }

    // ── IAgent-compatible interface ───────────────────────────────────────────

    /// Return the list of capability names (one per MCP tool).
    ///
    /// Each string and the outer slice are heap-allocated from `allocator`;
    /// the caller is responsible for freeing them.
    pub fn getCapabilities(self: *McpPlugin, allocator: std.mem.Allocator) ![][]const u8 {
        const caps = try allocator.alloc([]const u8, self.tools.items.len);
        for (self.tools.items, 0..) |tool, i| {
            caps[i] = try allocator.dupe(u8, tool.name);
        }
        return caps;
    }

    /// Dispatch a Borgkit request to the matching MCP tool and return a response.
    ///
    /// Steps:
    ///   1. Find the tool by req.capability.
    ///   2. Build a tools/call JSON-RPC request (req.payload used as arguments).
    ///   3. Send via the configured transport.
    ///   4. Extract result.content[0].text from the response.
    ///   5. Return AgentResponse.success with the extracted text.
    pub fn handleRequest(
        self: *McpPlugin,
        req: types.AgentRequest,
        allocator: std.mem.Allocator,
    ) !types.AgentResponse {
        // Verify the tool exists.
        var found = false;
        for (self.tools.items) |t| {
            if (std.mem.eql(u8, t.name, req.capability)) { found = true; break; }
        }
        if (!found) return types.AgentResponse.err(req.request_id, "MCP tool not found");

        // JSON-encode the tool name safely.
        var name_json = std.ArrayList(u8).init(allocator);
        defer name_json.deinit();
        try std.json.encodeJsonString(req.capability, .{}, name_json.writer());

        // Use the request payload as arguments; fall back to empty object.
        const args = if (req.payload.len == 0) "{}" else req.payload;

        const params = try std.fmt.allocPrint(
            allocator,
            "{{\"name\":{s},\"arguments\":{s}}}",
            .{ name_json.items, args },
        );
        defer allocator.free(params);

        const rpc_req = try self.buildRequest("tools/call", params, allocator);
        defer allocator.free(rpc_req);

        const raw = switch (self.mode) {
            .stdio => try self.sendStdioRequest(rpc_req, allocator),
            .http  => try self.sendHttpRequest(rpc_req, allocator),
        };
        defer allocator.free(raw);

        const text = extractContentText(raw, allocator) catch |e| blk: {
            std.log.warn("[mcp_plugin] content extract error: {} — returning raw response", .{e});
            break :blk try allocator.dupe(u8, raw);
        };
        defer allocator.free(text);

        return types.AgentResponse.success(req.request_id, text);
    }

    // ── private wire helpers ──────────────────────────────────────────────────

    /// Write `request_json` to the child's stdin and read back one response line.
    ///
    /// Returned slice is owned by `allocator`; caller must free it.
    fn sendStdioRequest(
        self: *McpPlugin,
        request_json: []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const child  = &(self.child  orelse return error.NoChildProcess);
        const stdin  = child.stdin   orelse return error.NoPipe;
        const stdout = child.stdout  orelse return error.NoPipe;

        try stdin.writeAll(request_json);

        return stdout.reader().readUntilDelimiterAlloc(allocator, '\n', MAX_LINE);
    }

    /// POST `request_json` to the configured HTTP URL and return the response body.
    ///
    /// Returned slice is owned by `allocator`; caller must free it.
    fn sendHttpRequest(
        self: *McpPlugin,
        request_json: []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const client = &(self.http_client orelse return error.NoHttpClient);
        const config =   self.http_config  orelse return error.NoHttpConfig;

        // Strip the trailing newline for HTTP; it is only needed for stdio framing.
        const body = std.mem.trimRight(u8, request_json, "\n");

        var hdrs_buf: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs_buf[n] = .{ .name = "Content-Type", .value = "application/json" }; n += 1;
        hdrs_buf[n] = .{ .name = "Accept",        .value = "application/json" }; n += 1;
        if (config.api_key) |key| {
            hdrs_buf[n] = .{ .name = "Authorization", .value = key }; n += 1;
        }

        var resp_body = std.ArrayList(u8).init(allocator);
        errdefer resp_body.deinit();

        const fr = try client.fetch(.{
            .method           = .POST,
            .location         = .{ .url = config.url },
            .extra_headers    = hdrs_buf[0..n],
            .payload          = body,
            .response_storage = .{ .dynamic = &resp_body },
        });

        const code = @intFromEnum(fr.status);
        if (code < 200 or code >= 300) {
            std.log.err("[mcp_plugin] HTTP {d} from {s}", .{ code, config.url });
            return error.McpHttpError;
        }

        return resp_body.toOwnedSlice();
    }

    /// Build a JSON-RPC 2.0 request frame and append a newline:
    ///   {"jsonrpc":"2.0","id":N,"method":"METHOD","params":PARAMS}\n
    ///
    /// Increments self.next_id.
    /// Returned slice is owned by `allocator`; caller must free it.
    fn buildRequest(
        self: *McpPlugin,
        method: []const u8,
        params_json: []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const id = self.next_id;
        self.next_id += 1;
        return std.fmt.allocPrint(
            allocator,
            "{{\"jsonrpc\":\"2.0\",\"id\":{d},\"method\":\"{s}\",\"params\":{s}}}\n",
            .{ id, method, params_json },
        );
    }
};

// ── module-level helpers ──────────────────────────────────────────────────────

/// Parse a tools/call JSON-RPC response and return a copy of
/// result.content[0].text.
///
/// Uses a temporary arena for parsing; the returned string is allocated from
/// `allocator` and must be freed by the caller.
fn extractContentText(raw: []const u8, allocator: std.mem.Allocator) ![]const u8 {
    var arena = std.heap.ArenaAllocator.init(allocator);
    defer arena.deinit();

    const parsed = try std.json.parseFromSlice(
        std.json.Value, arena.allocator(), raw, .{ .ignore_unknown_fields = true },
    );

    // Propagate JSON-RPC errors as a readable string rather than crashing.
    if (parsed.value == .object) {
        if (parsed.value.object.get("error")) |err_val| {
            const msg: []const u8 = blk: {
                if (err_val == .object) {
                    if (err_val.object.get("message")) |mv| {
                        if (mv == .string) break :blk mv.string;
                    }
                }
                break :blk "MCP error (no message)";
            };
            return allocator.dupe(u8, msg);
        }
    }

    // Navigate: result → content → [0] → text
    const result_obj = switch (parsed.value) {
        .object => |o| o.get("result") orelse return error.NoResult,
        else    => return error.NoResult,
    };
    const content_val = switch (result_obj) {
        .object => |o| o.get("content") orelse return error.NoContent,
        else    => return error.NoContent,
    };
    const items = switch (content_val) {
        .array => |a| a.items,
        else   => return error.ContentNotArray,
    };
    if (items.len == 0) return error.EmptyContent;

    const first_obj = switch (items[0]) {
        .object => |o| o,
        else    => return error.ContentItemNotObject,
    };
    const text_val = first_obj.get("text") orelse return error.NoTextField;
    const text = switch (text_val) {
        .string => |s| s,
        else    => return error.TextNotString,
    };
    return allocator.dupe(u8, text);
}
