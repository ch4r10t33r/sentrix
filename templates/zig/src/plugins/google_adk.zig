//! Google ADK → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a running Google ADK web server (`adk web`) so it is discoverable
//! and callable on the Borgkit mesh as a standard IAgent.
//!
//! ── Google ADK HTTP API ────────────────────────────────────────────────────────
//!
//!   POST /run
//!     Body: {
//!       "app_name":    "my_app",
//!       "user_id":     "borgkit-user",
//!       "session_id":  "<uuid>",
//!       "new_message": { "role": "user", "parts": [{ "text": "..." }] }
//!     }
//!     Response: [ Event, ... ]   (array of ADK Event objects)
//!
//! Start the ADK server with:
//!   adk web my_agent_package/ --port 8080
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const adk = @import("plugins/google_adk.zig");
//!
//!   var service = adk.GoogleADKService{
//!       .base_url  = "http://localhost:8080",
//!       .app_name  = "my_agent",
//!   };
//!   var plugin = adk.GoogleADKPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(adk.GoogleADKService, adk.GoogleADKPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/gemini",
//!       .owner    = "0xYourWallet",
//!   }, allocator);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config ─────────────────────────────────────────────────────────────

/// Configuration for a Google ADK (`adk web`) HTTP endpoint.
pub const GoogleADKService = struct {
    /// Base URL of the ADK web server, e.g. "http://localhost:8080".
    base_url: []const u8 = "http://localhost:8080",

    /// ADK application name (must match the agent package name).
    app_name: []const u8 = "agent",

    /// User ID passed to every ADK session.
    user_id: []const u8 = "borgkit-user",

    /// POST path for running the agent (default: "/run").
    run_route: []const u8 = "/run",

    /// Optional Bearer token for the Authorization header.
    api_key: ?[]const u8 = null,
};

// ── Plugin ─────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for Google ADK.
pub const GoogleADKPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,
    /// Monotonically incrementing counter used as a lightweight session seed.
    _seq:      u64,

    pub fn init(allocator: std.mem.Allocator) GoogleADKPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
            ._seq      = 0,
        };
    }

    pub fn deinit(self: *GoogleADKPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    pub fn extractCapabilities(
        _self:     *GoogleADKPlugin,
        _agent:    *GoogleADKService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the Google ADK agent",
        };
        return caps;
    }

    /// Build the ADK `/run` request body.
    ///
    /// Extracts `message`, `input`, or `query` from the Borgkit payload; falls
    /// back to the raw JSON payload as the message text.
    ///
    /// A unique session ID is generated per call (sequential counter) so each
    /// Borgkit request is independently stateless from ADK's perspective.
    pub fn translateRequest(
        self:      *GoogleADKPlugin,
        req:       types.AgentRequest,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Increment session counter atomically
        const seq = @atomicRmw(u64, &self._seq, .Add, 1, .monotonic);

        const message = blk: {
            const parsed = std.json.parseFromSlice(
                std.json.Value, allocator, req.payload, .{ .ignore_unknown_fields = true },
            ) catch break :blk try allocator.dupe(u8, req.payload);
            defer parsed.deinit();

            if (parsed.value == .object) {
                for ([_][]const u8{ "message", "input", "query" }) |key| {
                    if (parsed.value.object.get(key)) |v| {
                        if (v == .string) {
                            break :blk try allocator.dupe(u8, v.string);
                        }
                    }
                }
            }
            break :blk try allocator.dupe(u8, req.payload);
        };
        defer allocator.free(message);

        var escaped_msg = std.ArrayList(u8).init(allocator);
        defer escaped_msg.deinit();
        try std.json.encodeJsonString(message, .{}, escaped_msg.writer());

        var escaped_req_id = std.ArrayList(u8).init(allocator);
        defer escaped_req_id.deinit();
        try std.json.encodeJsonString(req.request_id, .{}, escaped_req_id.writer());

        // Session ID: combine request_id prefix and seq for uniqueness
        const session_id = try std.fmt.allocPrint(
            allocator, "borgkit-{s}-{d}", .{ req.request_id[0..@min(8, req.request_id.len)], seq },
        );
        defer allocator.free(session_id);

        var escaped_session = std.ArrayList(u8).init(allocator);
        defer escaped_session.deinit();
        try std.json.encodeJsonString(session_id, .{}, escaped_session.writer());

        // app_name and user_id are injected at invoke time from GoogleADKService;
        // we use placeholder empty strings here and replace them in invokeNative.
        return std.fmt.allocPrint(allocator,
            \\{{"app_name":"__APP__","user_id":"__USER__","session_id":{s},"new_message":{{"role":"user","parts":[{{"text":{s}}}]}},"__req_id__":{s}}}
            , .{ escaped_session.items, escaped_msg.items, escaped_req_id.items });
    }

    /// Extract readable text from the ADK Event array.
    ///
    /// Collects all `event.content.parts[*].text` values and joins them.
    pub fn translateResponse(
        _self:      *GoogleADKPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractAdkText(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// Inject `app_name` / `user_id` from service config, then POST to
    /// `{base_url}{run_route}`.
    pub fn invokeNative(
        self:      *GoogleADKPlugin,
        agent:     *GoogleADKService,
        input:     []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Substitute __APP__ and __USER__ placeholders with real service values
        const with_app  = try std.mem.replaceOwned(u8, allocator, input,  "__APP__",  agent.app_name);
        defer allocator.free(with_app);
        const with_user = try std.mem.replaceOwned(u8, allocator, with_app, "__USER__", agent.user_id);
        defer allocator.free(with_user);

        // Strip the internal __req_id__ field before sending
        // (simple approach: the server ignores unknown keys, so we just send as-is)
        const url = try std.fmt.allocPrint(
            allocator, "{s}{s}",
            .{ trimSlash(agent.base_url), agent.run_route },
        );
        defer allocator.free(url);

        return self.postJson(agent, url, with_user, allocator);
    }

    // ── HTTP helper ────────────────────────────────────────────────────────

    fn postJson(
        self:      *GoogleADKPlugin,
        agent:     *GoogleADKService,
        url:       []const u8,
        payload:   []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        var hdrs_buf: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs_buf[n] = .{ .name = "Content-Type", .value = "application/json" };
        n += 1;
        hdrs_buf[n] = .{ .name = "Accept",       .value = "application/json" };
        n += 1;
        if (agent.api_key) |key| {
            hdrs_buf[n] = .{ .name = "Authorization", .value = key };
            n += 1;
        }

        var body = std.ArrayList(u8).init(allocator);
        errdefer body.deinit();

        const fr = try self.client.fetch(.{
            .method           = .POST,
            .location         = .{ .url = url },
            .extra_headers    = hdrs_buf[0..n],
            .payload          = payload,
            .response_storage = .{ .dynamic = &body },
        });

        const code = @intFromEnum(fr.status);
        if (code < 200 or code >= 300) return error.GoogleADKBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Extract and join all `event.content.parts[*].text` values from an ADK
/// Event array JSON string.
fn extractAdkText(raw: []const u8) ?[]const u8 {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const parsed = std.json.parseFromSlice(
        std.json.Value, alloc, raw, .{ .ignore_unknown_fields = true },
    ) catch return null;

    const events = switch (parsed.value) {
        .array => |a| a.items,
        else   => return null,
    };

    var parts = std.ArrayList(u8).init(alloc);
    for (events) |event| {
        const content = switch (event) {
            .object => |o| o.get("content") orelse continue,
            else    => continue,
        };
        const ps = switch (content) {
            .object => |o| (o.get("parts") orelse continue),
            else    => continue,
        };
        const arr = switch (ps) {
            .array => |a| a.items,
            else   => continue,
        };
        for (arr) |part| {
            const text = switch (part) {
                .object => |o| (o.get("text") orelse continue),
                else    => continue,
            };
            if (text == .string and text.string.len > 0) {
                if (parts.items.len > 0) parts.append('\n') catch {};
                parts.appendSlice(text.string) catch {};
            }
        }
    }

    if (parts.items.len == 0) return null;
    // Copy out of the arena before it is freed
    const result = std.heap.page_allocator.dupe(u8, parts.items) catch return null;
    return result;
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
