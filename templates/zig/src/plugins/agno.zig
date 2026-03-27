//! Agno → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a deployed Agno FastAPI agent server so it participates in the
//! Borgkit mesh as a standard IAgent.  Uses `std.http.Client` for all HTTP
//! communication — no external dependencies required.
//!
//! ── Agno API contract ─────────────────────────────────────────────────────────
//!
//!   POST {base_url}/run                          (default, single-agent)
//!     Body:   { "message": "...", "stream": false }
//!     Response: { "content": "...", "messages": [...] }
//!
//!   POST {base_url}/v1/agents/{agent_id}/runs    (when agent_id is set)
//!     Same body / response shape.
//!
//! The plugin tries `content` first in the response; if absent it falls back
//! to the `content` field of the last entry in the `messages` array.
//!
//! ── Setup ──────────────────────────────────────────────────────────────────────
//!
//!   Start an Agno FastAPI server:
//!     from agno.agent import Agent
//!     from agno.playground import Playground
//!     agent = Agent(...)
//!     app   = Playground(agents=[agent]).get_app()
//!     # uvicorn app:app --port 7777
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const agno = @import("plugins/agno.zig");
//!
//!   var service = agno.AgnoService{
//!       .base_url  = "http://localhost:7777",
//!       .agent_id  = "my_agent",   // optional
//!   };
//!   var plugin = agno.AgnoPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(agno.AgnoService, agno.AgnoPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/agno",
//!       .owner    = "0xYourWallet",
//!   }, allocator);
//!
//!   const resp = agent.handleRequest(req);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config (the "agent" token) ────────────────────────────────────────

/// Configuration for an Agno FastAPI HTTP endpoint.
pub const AgnoService = struct {
    /// Base URL, e.g. "http://localhost:7777".
    base_url: []const u8 = "http://localhost:7777",

    /// POST path used when agent_id is not set (default: "/run").
    invoke_route: []const u8 = "/run",

    /// Optional Agno agent ID.  When set the plugin posts to
    /// `{base_url}/v1/agents/{agent_id}/runs` instead of `{base_url}{invoke_route}`.
    agent_id: ?[]const u8 = null,

    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,
};

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for Agno FastAPI agent servers.
///
/// Stores a reusable `std.http.Client`; call `deinit()` when done.
pub const AgnoPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) AgnoPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *AgnoPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return a single "invoke" capability — capabilities are static for HTTP
    /// bridge plugins.  Pass explicit capabilities at wrap time via
    /// `WrappedAgent.init` if you need multiple.
    pub fn extractCapabilities(
        _self:     *AgnoPlugin,
        _agent:    *AgnoService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the Agno agent via its FastAPI HTTP server",
        };
        return caps;
    }

    /// Build the Agno run JSON body:
    ///   { "message": <payload>, "stream": false }
    ///
    /// Extracts `message`, `input`, or `query` key from the request payload if
    /// present; otherwise uses the raw payload string as the message.
    pub fn translateRequest(
        _self:     *AgnoPlugin,
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
            \\{{"message":{s},"stream":false}}
            , .{escaped.items});
    }

    /// Extract the agent reply from the Agno response.
    ///
    /// Tries `content` at the top level first; if absent or empty, walks
    /// `messages` in reverse and returns the first non-empty `content` value
    /// found.  Falls back to the raw JSON when nothing matches.
    pub fn translateResponse(
        _self:      *AgnoPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractAgnoContent(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// POST the translated body to the appropriate Agno URL and return the
    /// raw response bytes.
    ///
    /// URL selection:
    ///   • agent_id set → `{base_url}/v1/agents/{agent_id}/runs`
    ///   • otherwise    → `{base_url}{invoke_route}`
    pub fn invokeNative(
        self:      *AgnoPlugin,
        agent:     *AgnoService,
        input:     []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const url = if (agent.agent_id) |id|
            try std.fmt.allocPrint(
                allocator, "{s}/v1/agents/{s}/runs",
                .{ trimSlash(agent.base_url), id },
            )
        else
            try std.fmt.allocPrint(
                allocator, "{s}{s}",
                .{ trimSlash(agent.base_url), agent.invoke_route },
            );
        defer allocator.free(url);

        return self.postJson(agent, url, input, allocator);
    }

    // ── Internal HTTP helper ────────────────────────────────────────────────

    fn postJson(
        self:      *AgnoPlugin,
        agent:     *AgnoService,
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
        if (code < 200 or code >= 300) return error.AgnoBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Try to extract a meaningful text reply from an Agno response JSON.
///
/// Strategy:
///   1. Return `content` at the top level if it is a non-empty string.
///   2. Walk `messages` in reverse; return the first non-empty `content`.
///   3. Return null so the caller falls back to raw bytes.
fn extractAgnoContent(raw: []const u8) ?[]const u8 {
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

    // 1. Top-level "content" field
    if (root.get("content")) |v| {
        if (v == .string and v.string.len > 0) return v.string;
    }

    // 2. Last non-empty content in messages array
    const messages_val = root.get("messages") orelse return null;
    const arr = switch (messages_val) {
        .array => |a| a.items,
        else   => return null,
    };

    var i = arr.len;
    while (i > 0) {
        i -= 1;
        const msg = arr[i];
        if (msg != .object) continue;
        const content = msg.object.get("content") orelse continue;
        if (content == .string and content.string.len > 0) return content.string;
    }

    return null;
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
