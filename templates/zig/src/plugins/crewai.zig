//! CrewAI → Borgkit Plugin (Zig) — HTTP Bridge
//!
//! Wraps a running CrewAI HTTP service so Zig agents on the Borgkit mesh can
//! invoke CrewAI crews without embedding a Python interpreter.
//!
//! ── Expected service endpoints ─────────────────────────────────────────────────
//!
//!   GET  /capabilities
//!     → [{ "name": "...", "description": "...", "parameters"?: {...} }]
//!
//!   POST /kickoff
//!     Body:     { "capability"?: "...", "task"?: "...", "inputs": {...} }
//!     Response: { "result": "...", "status": "success"|"error", "error"?: "..." }
//!
//! ── Serving CrewAI over HTTP ───────────────────────────────────────────────────
//!
//!   # serve_crew.py  (FastAPI wrapper — drop next to your crew)
//!   from fastapi import FastAPI
//!   from my_crew import my_crew
//!
//!   app = FastAPI()
//!
//!   @app.get("/capabilities")
//!   def caps():
//!       return [{"name": "kickoff", "description": "Run the crew on a task"}]
//!
//!   @app.post("/kickoff")
//!   async def kickoff(body: dict):
//!       result = my_crew.kickoff(inputs=body.get("inputs", {}))
//!       return {"result": str(result), "status": "success"}
//!
//!   # uvicorn serve_crew:app --port 8000
//!
//! ── Usage ──────────────────────────────────────────────────────────────────────
//!
//!   const crewai = @import("plugins/crewai.zig");
//!
//!   var service = crewai.CrewAIService{
//!       .base_url = "http://localhost:8000",
//!   };
//!   var plugin = crewai.CrewAIPlugin.init(allocator);
//!   defer plugin.deinit();
//!
//!   const Wrapped = wrapped_agent.WrappedAgent(crewai.CrewAIService, crewai.CrewAIPlugin);
//!   var agent = Wrapped.init(&service, &plugin, .{
//!       .agent_id = "borgkit://agent/writer-crew",
//!       .owner    = "0xYourWallet",
//!   }, allocator);

const std     = @import("std");
const types   = @import("../types.zig");
const iplugin = @import("iPlugin.zig");

// ── Service config ─────────────────────────────────────────────────────────────

/// Configuration for a CrewAI HTTP service endpoint.
pub const CrewAIService = struct {
    /// Base URL of the service, e.g. "http://localhost:8000".
    base_url: []const u8 = "http://localhost:8000",

    /// POST path for crew execution (default: "/kickoff").
    kickoff_route: []const u8 = "/kickoff",

    /// GET path for capability discovery (default: "/capabilities").
    capabilities_route: []const u8 = "/capabilities",

    /// Optional Bearer token for the Authorization header.
    api_key: ?[]const u8 = null,
};

// ── Plugin ─────────────────────────────────────────────────────────────────────

/// HTTP bridge plugin for CrewAI.
pub const CrewAIPlugin = struct {
    allocator: std.mem.Allocator,
    client:    std.http.Client,

    pub fn init(allocator: std.mem.Allocator) CrewAIPlugin {
        return .{
            .allocator = allocator,
            .client    = std.http.Client{ .allocator = allocator },
        };
    }

    pub fn deinit(self: *CrewAIPlugin) void {
        self.client.deinit();
    }

    // ── IPlugin interface ───────────────────────────────────────────────────

    /// Return capabilities fetched from GET /capabilities, or a single
    /// "invoke" fallback when the endpoint is unavailable.
    pub fn extractCapabilities(
        self:      *CrewAIPlugin,
        agent:     *CrewAIService,
        allocator: std.mem.Allocator,
    ) []iplugin.CapabilityDescriptor {
        return self.fetchCapabilities(agent, allocator) catch {
            // Remote fetch failed — fall back to a single "invoke" capability
            const caps = allocator.alloc(iplugin.CapabilityDescriptor, 1) catch
                return &[_]iplugin.CapabilityDescriptor{};
            caps[0] = .{
                .name        = "invoke",
                .description = "Invoke the CrewAI crew",
            };
            return caps;
        };
    }

    /// GET /capabilities and parse the JSON array into CapabilityDescriptors.
    pub fn fetchCapabilities(
        self:      *CrewAIPlugin,
        agent:     *CrewAIService,
        allocator: std.mem.Allocator,
    ) ![]iplugin.CapabilityDescriptor {
        const url = try std.fmt.allocPrint(
            allocator, "{s}{s}",
            .{ trimSlash(agent.base_url), agent.capabilities_route },
        );
        defer allocator.free(url);

        const raw = try self.doRequest(.GET, agent, url, null, allocator);
        defer allocator.free(raw);

        return parseCapabilities(raw, allocator);
    }

    /// Build the CrewAI kickoff JSON body from a Borgkit `AgentRequest`.
    ///
    /// The `task`, `query`, or `input` payload key (first found) becomes the
    /// `task` field; the full payload is forwarded as `inputs`.
    pub fn translateRequest(
        _self:     *CrewAIPlugin,
        req:       types.AgentRequest,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        // Extract task description from payload
        const task_opt: ?[]const u8 = blk: {
            const parsed = std.json.parseFromSlice(
                std.json.Value, allocator, req.payload, .{ .ignore_unknown_fields = true },
            ) catch break :blk null;
            defer parsed.deinit();

            if (parsed.value == .object) {
                for ([_][]const u8{ "task", "query", "input" }) |key| {
                    if (parsed.value.object.get(key)) |v| {
                        if (v == .string) {
                            break :blk try allocator.dupe(u8, v.string);
                        }
                    }
                }
            }
            break :blk null;
        };
        defer if (task_opt) |t| allocator.free(t);

        var escaped_cap = std.ArrayList(u8).init(allocator);
        defer escaped_cap.deinit();
        try std.json.encodeJsonString(req.capability, .{}, escaped_cap.writer());

        if (task_opt) |task| {
            var escaped_task = std.ArrayList(u8).init(allocator);
            defer escaped_task.deinit();
            try std.json.encodeJsonString(task, .{}, escaped_task.writer());

            return std.fmt.allocPrint(allocator,
                \\{{"capability":{s},"task":{s},"inputs":{s}}}
                , .{ escaped_cap.items, escaped_task.items, req.payload });
        } else {
            return std.fmt.allocPrint(allocator,
                \\{{"capability":{s},"inputs":{s}}}
                , .{ escaped_cap.items, req.payload });
        }
    }

    /// Map a CrewAI service response back to an `AgentResponse`.
    ///
    /// Handles `{ "result": "...", "status": "..." }` and plain string bodies.
    pub fn translateResponse(
        _self:      *CrewAIPlugin,
        request_id: []const u8,
        raw:        []const u8,
    ) types.AgentResponse {
        // Try to parse and extract result / error
        var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const parsed = std.json.parseFromSlice(
            std.json.Value, alloc, raw, .{ .ignore_unknown_fields = true },
        ) catch return types.AgentResponse.success(request_id, raw);

        if (parsed.value == .object) {
            const obj = parsed.value.object;

            // Check for error status
            const status = if (obj.get("status")) |s| (if (s == .string) s.string else "") else "";
            if (std.mem.eql(u8, status, "error")) {
                const err_msg = if (obj.get("error")) |e| (if (e == .string) e.string else raw) else raw;
                return types.AgentResponse.err(request_id, err_msg);
            }

            // Extract result or output
            for ([_][]const u8{ "result", "output" }) |key| {
                if (obj.get(key)) |v| {
                    const content = if (v == .string) v.string else raw;
                    return types.AgentResponse.success(request_id, content);
                }
            }
        }

        return types.AgentResponse.success(request_id, raw);
    }

    /// POST the translated body to `{base_url}{kickoff_route}`.
    pub fn invokeNative(
        self:      *CrewAIPlugin,
        agent:     *CrewAIService,
        input:     []const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        const url = try std.fmt.allocPrint(
            allocator, "{s}{s}",
            .{ trimSlash(agent.base_url), agent.kickoff_route },
        );
        defer allocator.free(url);

        return self.doRequest(.POST, agent, url, input, allocator);
    }

    // ── HTTP helper ─────────────────────────────────────────────────────────

    fn doRequest(
        self:      *CrewAIPlugin,
        method:    std.http.Method,
        agent:     *CrewAIService,
        url:       []const u8,
        payload:   ?[]const u8,
        allocator: std.mem.Allocator,
    ) ![]const u8 {
        var hdrs_buf: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs_buf[n] = .{ .name = "Accept", .value = "application/json" };
        n += 1;
        if (payload != null) {
            hdrs_buf[n] = .{ .name = "Content-Type", .value = "application/json" };
            n += 1;
        }
        if (agent.api_key) |key| {
            hdrs_buf[n] = .{ .name = "Authorization", .value = key };
            n += 1;
        }

        var body = std.ArrayList(u8).init(allocator);
        errdefer body.deinit();

        const fr = try self.client.fetch(.{
            .method           = method,
            .location         = .{ .url = url },
            .extra_headers    = hdrs_buf[0..n],
            .payload          = payload,
            .response_storage = .{ .dynamic = &body },
        });

        const code = @intFromEnum(fr.status);
        if (code < 200 or code >= 300) return error.CrewAIBadStatus;

        return body.toOwnedSlice();
    }
};

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Parse a JSON array of `{ name, description }` objects into CapabilityDescriptors.
fn parseCapabilities(
    raw:       []const u8,
    allocator: std.mem.Allocator,
) ![]iplugin.CapabilityDescriptor {
    const parsed = try std.json.parseFromSlice(
        std.json.Value, allocator, raw, .{ .ignore_unknown_fields = true },
    );
    defer parsed.deinit();

    const arr = switch (parsed.value) {
        .array => |a| a.items,
        else   => return error.NotAnArray,
    };

    var caps = try std.ArrayList(iplugin.CapabilityDescriptor).initCapacity(allocator, arr.len);
    errdefer caps.deinit();

    for (arr) |item| {
        const obj = switch (item) {
            .object => |o| o,
            else    => continue,
        };

        const name = if (obj.get("name")) |v| (if (v == .string) v.string else continue) else continue;
        const desc = if (obj.get("description")) |v| (if (v == .string) v.string else "") else "";

        try caps.append(.{
            .name        = try allocator.dupe(u8, name),
            .description = try allocator.dupe(u8, desc),
        });
    }

    return caps.toOwnedSlice();
}

fn trimSlash(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0 and s[end - 1] == '/') end -= 1;
    return s[0..end];
}
