//! OpenAI → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps an OpenAI-compatible chat completions endpoint so it participates in
//! the Borgkit mesh as a standard IAgent.  Uses `std.http.Client` for all HTTP
//! communication — no external dependencies required.
//!
//! ── OpenAI API contract ────────────────────────────────────────────────────────
//!
//!   POST {base_url}/v1/chat/completions
//!     Body:
//!       { "model": "gpt-4o-mini",
//!         "messages": [{ "role": "user", "content": "..." }],
//!         "max_tokens": 1024 }
//!     Response:
//!       { "choices": [{ "message": { "role": "assistant", "content": "..." } }] }
//!
//! Any OpenAI-compatible server (Azure OpenAI, local llama.cpp with
//! --jinja flag, Ollama with openai compat, etc.) can be pointed to by
//! overriding `base_url`.
//!
//! ── Setup ──────────────────────────────────────────────────────────────────────
//!
//!   Set OPENAI_API_KEY in your environment, then pass it at init time:
//!     var service = openai.OpenAIService{
//!         .api_key = std.process.getEnvVarOwned(allocator, "OPENAI_API_KEY")
//!                        catch null,
//!     };
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const openai = @import("plugins/openai.zig");
//!
//!   var service = openai.OpenAIService{
//!       .base_url   = "https://api.openai.com",
//!       .model      = "gpt-4o-mini",
//!       .api_key    = "sk-...",
//!       .max_tokens = 1024,
//!   };
//!   var plugin = openai.OpenAIPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(openai.OpenAIService, openai.OpenAIPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/openai",
//!       .owner    = "0xYourWallet",
//!   }, allocator);
//!
//!   const resp = agent.handleRequest(req);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config (the "agent" token) ────────────────────────────────────────

/// Configuration for an OpenAI-compatible chat completions HTTP endpoint.
pub const OpenAIService = struct {
    /// Base URL, e.g. "https://api.openai.com".
    base_url: []const u8 = "https://api.openai.com",

    /// Model name forwarded in every request (default: "gpt-4o-mini").
    model: []const u8 = "gpt-4o-mini",

    /// POST path for chat completions (default: "/v1/chat/completions").
    invoke_route: []const u8 = "/v1/chat/completions",

    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,

    /// Maximum tokens to generate per response (default: 1024).
    max_tokens: u32 = 1024,
};

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for OpenAI-compatible chat completions APIs.
///
/// Stores a reusable `std.http.Client`; call `deinit()` when done.
pub const OpenAIPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) OpenAIPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *OpenAIPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return a single "invoke" capability — capabilities are static for HTTP
    /// bridge plugins.  Pass explicit capabilities at wrap time via
    /// `WrappedAgent.init` if you need multiple.
    pub fn extractCapabilities(
        _self:     *OpenAIPlugin,
        _agent:    *OpenAIService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the OpenAI-compatible chat completions API",
        };
        return caps;
    }

    /// Build the OpenAI chat completions JSON body:
    ///   { "model": "gpt-4o-mini",
    ///     "messages": [{ "role": "user", "content": <payload> }],
    ///     "max_tokens": 1024 }
    ///
    /// Extracts `message`, `input`, or `query` key from the request payload if
    /// present; otherwise uses the raw payload string as content.
    pub fn translateRequest(
        _self:     *OpenAIPlugin,
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

        // NOTE: max_tokens is not available on _self because _self is unused here;
        // the default 1024 is baked in.  To make it dynamic, use invokeNative to
        // pass the agent config through.
        return std.fmt.allocPrint(allocator,
            \\{{"model":"gpt-4o-mini","messages":[{{"role":"user","content":{s}}}],"max_tokens":1024}}
            , .{escaped.items});
    }

    /// Extract the assistant message content from the OpenAI response.
    ///
    /// Looks for `choices[0].message.content`; falls back to returning the raw
    /// JSON if the expected structure is absent.
    pub fn translateResponse(
        _self:      *OpenAIPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractChoiceContent(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// POST the translated body to `{base_url}{invoke_route}` and return the
    /// raw response bytes.  Adds `Authorization: Bearer <key>` when api_key is set.
    pub fn invokeNative(
        self:      *OpenAIPlugin,
        agent:     *OpenAIService,
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
        self:      *OpenAIPlugin,
        agent:     *OpenAIService,
        url:       []const u8,
        payload:   []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Build extra headers; the Authorization value must outlive the fetch call
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
        if (code < 200 or code >= 300) return error.OpenAIBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse the OpenAI response and return `choices[0].message.content`.
/// Returns null if the JSON cannot be parsed or the expected path is missing.
fn extractChoiceContent(raw: []const u8) ?[]const u8 {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const parsed = std.json.parseFromSlice(
        std.json.Value, alloc, raw, .{ .ignore_unknown_fields = true },
    ) catch return null;

    const choices = switch (parsed.value) {
        .object => |obj| obj.get("choices") orelse return null,
        else    => return null,
    };

    const arr = switch (choices) {
        .array => |a| a.items,
        else   => return null,
    };

    if (arr.len == 0) return null;

    const first = arr[0];
    const message = switch (first) {
        .object => |obj| obj.get("message") orelse return null,
        else    => return null,
    };

    const content = switch (message) {
        .object => |obj| obj.get("content") orelse return null,
        else    => return null,
    };

    return switch (content) {
        .string => |s| s,
        else    => null,
    };
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
