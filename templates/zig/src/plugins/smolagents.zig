//! smolagents → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a smolagents/Gradio server so it participates in the Borgkit mesh as
//! a standard IAgent.  Supports both the standard Gradio predict API and a
//! custom task-oriented endpoint.  Uses `std.http.Client` for all HTTP
//! communication — no external dependencies required.
//!
//! ── API contract ──────────────────────────────────────────────────────────────
//!
//!   Gradio mode (use_gradio = true, default):
//!     POST {base_url}/run/predict
//!       Body:     { "data": ["..."] }
//!       Response: { "data": ["..."], "duration": 1.2 }
//!
//!   Custom mode (use_gradio = false):
//!     POST {base_url}/run
//!       Body:     { "task": "...", "kwargs": {} }
//!       Response: { "output": "..." }
//!
//! ── Setup ──────────────────────────────────────────────────────────────────────
//!
//!   Gradio / smolagents GradioUI:
//!     from smolagents import CodeAgent, HfApiModel, GradioUI
//!     agent = CodeAgent(tools=[], model=HfApiModel())
//!     GradioUI(agent).launch(server_port=7860)
//!
//!   Custom FastAPI wrapper:
//!     from smolagents import CodeAgent, HfApiModel
//!     from fastapi import FastAPI
//!     app   = FastAPI()
//!     agent = CodeAgent(tools=[], model=HfApiModel())
//!
//!     @app.post("/run")
//!     async def run(body: dict):
//!         return {"output": agent.run(body["task"])}
//!
//!     # uvicorn app:app --port 7860
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const smolagents = @import("plugins/smolagents.zig");
//!
//!   // Gradio mode (default)
//!   var service = smolagents.SmolagentsService{
//!       .base_url    = "http://localhost:7860",
//!       .use_gradio  = true,
//!   };
//!
//!   // Custom endpoint mode
//!   var service = smolagents.SmolagentsService{
//!       .base_url    = "http://localhost:7860",
//!       .invoke_route = "/run",
//!       .use_gradio  = false,
//!   };
//!
//!   var plugin = smolagents.SmolagentsPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(
//!       smolagents.SmolagentsService, smolagents.SmolagentsPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/smolagents",
//!       .owner    = "0xYourWallet",
//!   }, allocator);
//!
//!   const resp = agent.handleRequest(req);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config (the "agent" token) ────────────────────────────────────────

/// Configuration for a smolagents/Gradio HTTP endpoint.
pub const SmolagentsService = struct {
    /// Base URL, e.g. "http://localhost:7860".
    base_url: []const u8 = "http://localhost:7860",

    /// POST path used for invocation.
    /// Gradio default: "/run/predict".  Custom server default: "/run".
    invoke_route: []const u8 = "/run/predict",

    /// When true (default) the plugin uses the Gradio predict wire format:
    ///   request  → `{"data":["..."]}`,  response → `{"data":["..."]}`.
    /// When false a custom format is used:
    ///   request  → `{"task":"...","kwargs":{}}`,  response → `{"output":"..."}`.
    use_gradio: bool = true,

    /// Optional API key sent as "Authorization: Bearer <key>".
    api_key: ?[]const u8 = null,
};

// ── Plugin ────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for smolagents / Gradio servers.
///
/// Stores a reusable `std.http.Client`; call `deinit()` when done.
pub const SmolagentsPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) SmolagentsPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *SmolagentsPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return a single "invoke" capability — capabilities are static for HTTP
    /// bridge plugins.  Pass explicit capabilities at wrap time via
    /// `WrappedAgent.init` if you need multiple.
    pub fn extractCapabilities(
        _self:     *SmolagentsPlugin,
        _agent:    *SmolagentsService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
            return &[_]iplugin.CapabilityDescriptor{};
        caps[0] = .{
            .name        = "invoke",
            .description = "Invoke the smolagents agent via its Gradio or custom HTTP server",
        };
        return caps;
    }

    /// Build the request JSON body for the configured wire format.
    ///
    /// Gradio mode:  { "data": [<payload>] }
    /// Custom mode:  { "task": <payload>, "kwargs": {} }
    ///
    /// Extracts `message`, `input`, or `query` key from the request payload if
    /// present; otherwise uses the raw payload string as the task/message.
    pub fn translateRequest(
        _self:     *SmolagentsPlugin,
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

        // NOTE: _self is unused in translateRequest because use_gradio is a
        // service-level config accessed in invokeNative.  The Gradio format is
        // the default body; invokeNative may substitute the custom format based
        // on the live agent config.
        return std.fmt.allocPrint(allocator,
            \\{{"data":[{s}]}}
            , .{escaped.items});
    }

    /// Extract the agent reply from the server response.
    ///
    /// Gradio format: returns `data[0]` string.
    /// Custom format: returns the `output` field string.
    /// Falls back to the raw JSON when no recognised field is found.
    pub fn translateResponse(
        _self:      *SmolagentsPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        const content = extractSmolagentsOutput(raw) orelse raw;
        return types.AgentResponse.success(request_id, content);
    }

    /// POST the translated body to `{base_url}{invoke_route}` and return the
    /// raw response bytes.
    ///
    /// When `use_gradio` is false the request body is rebuilt using the custom
    /// `{"task":"...","kwargs":{}}` format before posting.
    pub fn invokeNative(
        self:      *SmolagentsPlugin,
        agent:     *SmolagentsService,
        input:     []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const url = try std.fmt.allocPrint(
            allocator, "{s}{s}",
            .{ trimSlash(agent.base_url), agent.invoke_route },
        );
        defer allocator.free(url);

        // When use_gradio is false we must re-wrap the payload as a task body.
        // `input` arrives as `{"data":["<content>"]}` from translateRequest;
        // we need to unwrap the first data element and re-encode as task format.
        if (!agent.use_gradio) {
            const task_body = try repackAsTask(input, allocator);
            defer allocator.free(task_body);
            return self.postJson(agent, url, task_body, allocator);
        }

        return self.postJson(agent, url, input, allocator);
    }

    // ── Internal HTTP helper ────────────────────────────────────────────────

    fn postJson(
        self:      *SmolagentsPlugin,
        agent:     *SmolagentsService,
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
        if (code < 200 or code >= 300) return error.SmolagentsBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Re-encode a Gradio-format body `{"data":["<task>"]}` as the custom task
/// format `{"task":"<task>","kwargs":{}}`.
///
/// Returns an allocator-owned slice; caller must free.
/// Falls back to forwarding the original `gradio_body` unchanged on parse error.
fn repackAsTask(gradio_body: []const u8, allocator: std.mem.Allocator) ![]const u8 {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const parsed = std.json.parseFromSlice(
        std.json.Value, alloc, gradio_body, .{ .ignore_unknown_fields = true },
    ) catch return allocator.dupe(u8, gradio_body);

    const data_val = switch (parsed.value) {
        .object => |obj| obj.get("data") orelse
            return allocator.dupe(u8, gradio_body),
        else => return allocator.dupe(u8, gradio_body),
    };

    const arr = switch (data_val) {
        .array => |a| a.items,
        else   => return allocator.dupe(u8, gradio_body),
    };

    if (arr.len == 0) return allocator.dupe(u8, gradio_body);

    const task_str = switch (arr[0]) {
        .string => |s| s,
        else    => return allocator.dupe(u8, gradio_body),
    };

    var escaped = std.ArrayList(u8).init(allocator);
    defer escaped.deinit();
    try std.json.encodeJsonString(task_str, .{}, escaped.writer());

    return std.fmt.allocPrint(allocator,
        \\{{"task":{s},"kwargs":{{}}}}
        , .{escaped.items});
}

/// Try to extract a text reply from a smolagents/Gradio response.
///
/// Strategy:
///   1. Gradio format: parse `data[0]` as a string.
///   2. Custom format: parse `output` as a string.
///   3. Return null so the caller falls back to raw bytes.
fn extractSmolagentsOutput(raw: []const u8) ?[]const u8 {
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

    // 1. Gradio: data[0]
    if (root.get("data")) |data_val| {
        if (data_val == .array) {
            const items = data_val.array.items;
            if (items.len > 0) {
                if (items[0] == .string) return items[0].string;
            }
        }
    }

    // 2. Custom: output
    if (root.get("output")) |v| {
        if (v == .string) return v.string;
    }

    return null;
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
