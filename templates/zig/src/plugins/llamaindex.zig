//! LlamaIndex → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a LlamaIndex chat/query server so it participates in the Borgkit mesh
//! as a standard IAgent.  Uses `std.http.Client` for all HTTP communication —
//! no external dependencies required.
//!
//! ── LlamaIndex API contract ───────────────────────────────────────────────────
//!
//!   POST {base_url}/chat
//!     Body:     { "message": "...", "chat_history": [] }
//!     Response: { "response": "...", "source_nodes": [] }
//!
//! The server is typically started via LlamaIndex's built-in FastAPI integration:
//!
//! ── Setup ──────────────────────────────────────────────────────────────────────
//!
//!   from llama_index.core import VectorStoreIndex, SimpleDirectoryReader
//!   from llama_index.server import LlamaIndexServer
//!
//!   index  = VectorStoreIndex.from_documents(
//!                SimpleDirectoryReader("data").load_data())
//!   engine = index.as_chat_engine()
//!   server = LlamaIndexServer(chat_engine=engine, port=8080)
//!   server.run()
//!
//!   # Or with llama-index-server:
//!   # llama-index-server --port 8080
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const llamaindex = @import("plugins/llamaindex.zig");
//!
//!   var service = llamaindex.LlamaIndexService{
//!       .base_url = "http://localhost:8080",
//!   };
//!   var plugin = llamaindex.LlamaIndexPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(
//!       llamaindex.LlamaIndexService, llamaindex.LlamaIndexPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/llamaindex",
//!       .owner    = "0xYourWallet",
//!   }, allocator);
//!
//!   const resp = agent.handleRequest(req);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config (the "agent" token) ────────────────────────────────────────

/// Configuration for a LlamaIndex HTTP chat endpoint.
pub const LlamaIndexService = struct {
    /// Base URL, e.g. "http://localhost:8080".
    base_url: []const u8 = "http://localhost:8080",

    /// POST path for chat invocation (default: "/chat").
    invoke_route: []const u8 = "/chat",

    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,
};

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for LlamaIndex chat servers.
///
/// Stores a reusable `std.http.Client`; call `deinit()` when done.
pub const LlamaIndexPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) LlamaIndexPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *LlamaIndexPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return a single "invoke" capability — capabilities are static for HTTP
    /// bridge plugins.  Pass explicit capabilities at wrap time via
    /// `WrappedAgent.init` if you need multiple.
    pub fn extractCapabilities(
        _self:     *LlamaIndexPlugin,
        _agent:    *LlamaIndexService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the LlamaIndex agent via its HTTP chat endpoint",
        };
        return caps;
    }

    /// Build the LlamaIndex chat JSON body:
    ///   { "message": <payload>, "chat_history": [] }
    ///
    /// Extracts `message`, `input`, or `query` key from the request payload if
    /// present; otherwise uses the raw payload string as the message.
    pub fn translateRequest(
        _self:     *LlamaIndexPlugin,
        req:       types.AgentRequest,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Try to extract a string content field from the JSON payload
        const content = blk: {
            const parsed = std.json.parseFromSlice(
                std.json.Value, allocator, req.payload, .{ .ignore_unknown_fields = true },
            ) catch break :blk req.payload;
            defer parsed.deinit();

            if (parsed.value == .object) {
                for ([_][]const u8{ "message", "input", "query" }) |key| {
                    if (parsed.value.object.get(key)) |v| {
                        if (v == .string) {
                            const copy = try allocator.dupe(u8, v.string);
                            break :blk copy;
                        }
                    }
                }
            }
            break :blk try allocator.dupe(u8, req.payload);
        };
        defer allocator.free(content);

        // JSON-encode the content string so it is safe inside the body
        var escaped = std.ArrayList(u8).init(allocator);
        defer escaped.deinit();
        try std.json.encodeJsonString(content, .{}, escaped.writer());

        return std.fmt.allocPrint(allocator,
            \\{{"message":{s},"chat_history":[]}}
            , .{escaped.items});
    }

    /// Extract the agent reply from the LlamaIndex response.
    ///
    /// Looks for the top-level `response` field; falls back to returning the
    /// raw JSON if the field is absent or not a string.
    pub fn translateResponse(
        _self:      *LlamaIndexPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractResponseField(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// POST the translated body to `{base_url}{invoke_route}` and return the
    /// raw response bytes.
    pub fn invokeNative(
        self:      *LlamaIndexPlugin,
        agent:     *LlamaIndexService,
        input:     []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const url = try std.fmt.allocPrint(
            allocator, "{s}{s}",
            .{ trimSlash(agent.base_url), agent.invoke_route },
        );
        defer allocator.free(url);

        return self.postJson(agent, url, input, allocator);
    }

    // ── Internal HTTP helper ────────────────────────────────────────────────

    fn postJson(
        self:      *LlamaIndexPlugin,
        agent:     *LlamaIndexService,
        url:       []const u8,
        payload:   []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        var auth_buf: [512]u8 = undefined;
        var hdrs_buf: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs_buf[n] = .{ .name = "Content-Type", .value = "application/json" };
        n += 1;
        hdrs_buf[n] = .{ .name = "Accept", .value = "application/json" };
        n += 1;
        if (agent.api_key) |key| {
            const bearer = try std.fmt.bufPrint(&auth_buf, "Bearer {s}", .{key});
            hdrs_buf[n] = .{ .name = "Authorization", .value = bearer };
            n += 1;
        }
        const extra = hdrs_buf[0..n];

        var body = std.ArrayList(u8).init(allocator);
        errdefer body.deinit();

        const fr = try self.client.fetch(.{
            .method           = .POST,
            .location         = .{ .url = url },
            .extra_headers    = extra,
            .payload          = payload,
            .response_storage = .{ .dynamic = &body },
        });

        const code = @intFromEnum(fr.status);
        if (code < 200 or code >= 300) return error.LlamaIndexBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a LlamaIndex response JSON and return the `response` field value.
/// Returns null if the JSON cannot be parsed or the field is not a string.
fn extractResponseField(raw: []const u8) ?[]const u8 {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const parsed = std.json.parseFromSlice(
        std.json.Value, alloc, raw, .{ .ignore_unknown_fields = true },
    ) catch return null;

    const root = switch (parsed.value) {
        .object => |obj| obj,
        else    => return null,
    };

    const response = root.get("response") orelse return null;
    return switch (response) {
        .string => |s| s,
        else    => null,
    };
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
