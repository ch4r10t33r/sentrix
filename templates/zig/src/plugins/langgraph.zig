//! LangGraph → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a LangServe / LangGraph Platform endpoint so it participates in the
//! Borgkit mesh as a standard IAgent.  Uses `std.http.Client` for all HTTP
//! communication — no external dependencies required.
//!
//! ── LangServe API contract ─────────────────────────────────────────────────────
//!
//!   POST {base_url}/invoke
//!     Body:
//!       { "input": { "messages": [{ "role": "human", "content": "..." }] },
//!         "config": { "recursion_limit": 25 } }
//!     Response:
//!       { "output": { "messages": [{ "type": "ai", "content": "..." }] } }
//!
//! Start a LangServe endpoint with:
//!   from langserve import add_routes
//!   add_routes(app, my_graph, path="/")
//!   # uvicorn app:app --port 8000
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const lg = @import("plugins/langgraph.zig");
//!
//!   var service = lg.LangGraphService{
//!       .base_url     = "http://localhost:8000",
//!       .invoke_route = "/invoke",
//!   };
//!   var plugin = lg.LangGraphPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(lg.LangGraphService, lg.LangGraphPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/researcher",
//!       .owner    = "0xYourWallet",
//!   }, allocator);
//!
//!   const resp = agent.handleRequest(req);

const std    = @import("std");
const types  = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config (the "agent" token) ────────────────────────────────────────

/// Configuration for a LangServe / LangGraph Platform HTTP endpoint.
pub const LangGraphService = struct {
    /// Base URL, e.g. "http://localhost:8000".
    base_url: []const u8 = "http://localhost:8000",

    /// POST path for single invocation (default: "/invoke").
    invoke_route: []const u8 = "/invoke",

    /// LangGraph recursion limit forwarded in every request (default: 25).
    recursion_limit: u32 = 25,

    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,
};

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for LangGraph / LangServe.
///
/// Stores a reusable `std.http.Client`; call `deinit()` when done.
pub const LangGraphPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) LangGraphPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *LangGraphPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return a single "invoke" capability — capabilities are static for HTTP
    /// bridge plugins.  Pass explicit capabilities at wrap time via
    /// `WrappedAgent.init` if you need multiple.
    pub fn extractCapabilities(
        _self:      *LangGraphPlugin,
        _agent:     *LangGraphService,
        allocator:  std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the LangGraph agent via LangServe",
        };
        return caps;
    }

    /// Build the LangServe JSON body:
    ///   { "input": { "messages": [{ "role": "human", "content": <payload> }] },
    ///     "config": { "recursion_limit": 25 } }
    ///
    /// Extracts `message`, `input`, or `query` key from the request payload if
    /// present; otherwise uses the raw payload string as content.
    pub fn translateRequest(
        _self:     *LangGraphPlugin,
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
            \\{{"input":{{"messages":[{{"role":"human","content":{s}}}]}},"config":{{"recursion_limit":25}}}}
            , .{escaped.items});
    }

    /// Extract the last AI / assistant message from the LangServe response.
    ///
    /// Looks for `output.messages[*]` with `type == "ai"` or `role == "assistant"`;
    /// falls back to returning the raw JSON if none found.
    pub fn translateResponse(
        _self:      *LangGraphPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractLastAiMessage(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// POST the translated body to `{base_url}{invoke_route}` and return the
    /// raw response bytes.
    pub fn invokeNative(
        self:      *LangGraphPlugin,
        agent:     *LangGraphService,
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
        self:      *LangGraphPlugin,
        agent:     *LangGraphService,
        url:       []const u8,
        payload:   []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Build extra headers
        var hdrs_buf: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs_buf[n] = .{ .name = "Content-Type", .value = "application/json" };
        n += 1;
        hdrs_buf[n] = .{ .name = "Accept", .value = "application/json" };
        n += 1;
        if (agent.api_key) |key| {
            hdrs_buf[n] = .{ .name = "Authorization", .value = key };
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
        if (code < 200 or code >= 300) return error.LangGraphBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Walk `output.messages` in reverse and return the first AI message content.
/// Returns null if the JSON cannot be parsed or no AI message is found.
fn extractLastAiMessage(raw: []const u8) ?[]const u8 {
    // Parse without allocator — we only inspect, never allocate sub-values
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const parsed = std.json.parseFromSlice(
        std.json.Value, alloc, raw, .{ .ignore_unknown_fields = true },
    ) catch return null;

    const output = switch (parsed.value) {
        .object => |obj| obj.get("output") orelse return null,
        else    => return null,
    };

    const messages = switch (output) {
        .object => |obj| obj.get("messages") orelse return null,
        else    => return null,
    };

    const arr = switch (messages) {
        .array => |a| a.items,
        else   => return null,
    };

    // Walk in reverse to find the last AI message
    var i = arr.len;
    while (i > 0) {
        i -= 1;
        const msg = arr[i];
        if (msg != .object) continue;
        const obj = msg.object;

        const type_  = if (obj.get("type"))  |v| (if (v == .string) v.string else "") else "";
        const role   = if (obj.get("role"))   |v| (if (v == .string) v.string else "") else "";

        const is_ai = std.mem.eql(u8, type_, "ai")
                   or std.mem.eql(u8, type_, "AIMessage")
                   or std.mem.eql(u8, role,  "assistant");

        if (is_ai) {
            const content = obj.get("content") orelse continue;
            return switch (content) {
                .string => |s| s,
                else    => null,
            };
        }
    }
    return null;
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
