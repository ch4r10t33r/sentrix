//! AgentClient — HTTP transport client for calling other Borgkit agents.
//!
//! Combines discovery (`LocalDiscovery`, `HttpDiscovery`, or `Libp2pDiscovery`) with
//! HTTP POST /invoke dispatch.
//!
//! `find` / `findById` / `findAll` return entries whose slices are **heap-allocated**;
//! free with `discovery.freeDiscoveryEntry` when done.
//!
//! Usage:
//!   var client = AgentClient.init(allocator, .{ .local = &discovery }, .{});
//!   defer client.deinit();
//!   const resp = try client.callCapability("weather_forecast", payload);

const std = @import("std");
const types = @import("types.zig");
const disc = @import("discovery.zig");
const disc_http = @import("discovery_http.zig");
const disc_libp2p = @import("discovery_libp2p.zig");

pub const DiscoveryRef = union(enum) {
    local: *disc.LocalDiscovery,
    http: *disc_http.HttpDiscovery,
    libp2p: *disc_libp2p.Libp2pDiscovery,

    fn query(
        self: DiscoveryRef,
        capability: []const u8,
        out: *std.ArrayList(types.DiscoveryEntry),
    ) !void {
        switch (self) {
            .local => |p| try p.query(capability, out),
            .http => |p| try p.query(capability, out),
            .libp2p => |p| try p.query(capability, out),
        }
    }

    fn findById(self: DiscoveryRef, allocator: std.mem.Allocator, agent_id: []const u8) !?types.DiscoveryEntry {
        switch (self) {
            .local => |p| {
                const e = p.findById(agent_id) orelse return null;
                return disc.cloneDiscoveryEntry(allocator, e);
            },
            .http => |p| return p.findById(agent_id),
            .libp2p => |p| return p.findById(agent_id),
        }
    }
};

pub const AgentClientOptions = struct {
    caller_id: []const u8 = "anonymous",
    timeout_ms: u64 = 30_000,
};

pub const AgentClient = struct {
    allocator: std.mem.Allocator,
    discovery: DiscoveryRef,
    options: AgentClientOptions,

    pub fn init(
        allocator: std.mem.Allocator,
        discovery: DiscoveryRef,
        options: AgentClientOptions,
    ) AgentClient {
        return .{ .allocator = allocator, .discovery = discovery, .options = options };
    }

    pub fn deinit(_: *AgentClient) void {}

    pub fn find(self: *AgentClient, capability: []const u8) !?types.DiscoveryEntry {
        var results = std.ArrayList(types.DiscoveryEntry).init(self.allocator);
        defer {
            switch (self.discovery) {
                .local => results.deinit(),
                .http, .libp2p => {
                    for (results.items) |e| disc.freeDiscoveryEntry(self.allocator, e);
                    results.deinit();
                },
            }
        }
        try self.discovery.query(capability, &results);
        for (results.items) |entry| {
            if (entry.health == .healthy) {
                return @as(?types.DiscoveryEntry, try disc.cloneDiscoveryEntry(self.allocator, entry));
            }
        }
        return if (results.items.len > 0)
            @as(?types.DiscoveryEntry, try disc.cloneDiscoveryEntry(self.allocator, results.items[0]))
        else
            null;
    }

    pub fn findAll(
        self: *AgentClient,
        capability: []const u8,
        out: *std.ArrayList(types.DiscoveryEntry),
    ) !void {
        var all = std.ArrayList(types.DiscoveryEntry).init(self.allocator);
        defer {
            switch (self.discovery) {
                .local => all.deinit(),
                .http, .libp2p => {
                    for (all.items) |e| disc.freeDiscoveryEntry(self.allocator, e);
                    all.deinit();
                },
            }
        }
        try self.discovery.query(capability, &all);
        for (all.items) |entry| {
            if (entry.health == .healthy) try out.append(try disc.cloneDiscoveryEntry(self.allocator, entry));
        }
        if (out.items.len == 0) {
            for (all.items) |entry| try out.append(try disc.cloneDiscoveryEntry(self.allocator, entry));
        }
    }

    pub fn findById(self: *AgentClient, agent_id: []const u8) !?types.DiscoveryEntry {
        return self.discovery.findById(self.allocator, agent_id);
    }

    pub fn callCapability(
        self: *AgentClient,
        capability: []const u8,
        payload: []const u8,
    ) !types.AgentResponse {
        const entry = (try self.find(capability)) orelse {
            std.log.err("[AgentClient] No agent found for capability: {s}", .{capability});
            return types.AgentResponse.err("no-agent", "No healthy agent found for capability");
        };
        defer disc.freeDiscoveryEntry(self.allocator, entry);
        return self.callEntry(entry, capability, payload);
    }

    pub fn call(
        self: *AgentClient,
        agent_id: []const u8,
        capability: []const u8,
        payload: []const u8,
    ) !types.AgentResponse {
        const entry = (try self.findById(agent_id)) orelse {
            std.log.err("[AgentClient] Agent not found: {s}", .{agent_id});
            return types.AgentResponse.err("no-agent", "Agent not found in discovery");
        };
        defer disc.freeDiscoveryEntry(self.allocator, entry);
        return self.callEntry(entry, capability, payload);
    }

    pub fn callEntry(
        self: *AgentClient,
        entry: types.DiscoveryEntry,
        capability: []const u8,
        payload: []const u8,
    ) !types.AgentResponse {
        const url = try self.endpointUrl(entry);
        defer self.allocator.free(url);
        return self.httpPost(url, capability, payload);
    }

    fn endpointUrl(self: *AgentClient, entry: types.DiscoveryEntry) ![]u8 {
        const scheme: []const u8 = if (entry.network.tls) "https" else "http";
        return std.fmt.allocPrint(
            self.allocator,
            "{s}://{s}:{d}/invoke",
            .{ scheme, entry.network.host, entry.network.port },
        );
    }

    fn httpPost(
        self: *AgentClient,
        url: []const u8,
        capability: []const u8,
        payload: []const u8,
    ) !types.AgentResponse {
        const body = try std.fmt.allocPrint(
            self.allocator,
            \\{{"requestId":"{s}","from":"{s}","capability":"{s}","payload":{s},"timestamp":{d}}}
            ,
            .{
                "req-zig-" ++ @typeName(@TypeOf(url))[0..4],
                self.options.caller_id,
                capability,
                payload,
                std.time.milliTimestamp(),
            },
        );
        defer self.allocator.free(body);

        var http_client = std.http.Client{ .allocator = self.allocator };
        defer http_client.deinit();

        var response_body = std.ArrayList(u8).init(self.allocator);
        defer response_body.deinit();

        const fr = http_client.fetch(.{
            .location = .{ .url = url },
            .method = .POST,
            .payload = body,
            .extra_headers = &.{
                .{ .name = "Content-Type", .value = "application/json" },
            },
            .response_storage = .{ .dynamic = &response_body },
        }) catch |err| {
            std.log.err("[AgentClient] HTTP POST to {s} failed: {}", .{ url, err });
            return types.AgentResponse.err("", "HTTP request failed");
        };

        _ = fr;
        const resp_str = response_body.items;
        if (std.mem.indexOf(u8, resp_str, "\"success\"") != null) {
            return types.AgentResponse.success("", resp_str);
        }
        if (std.mem.indexOf(u8, resp_str, "\"payment_required\"") != null) {
            std.log.warn("[AgentClient] payment_required — configure x402 wallet to auto-pay", .{});
        }
        return types.AgentResponse.err("", "error or payment_required from remote agent");
    }
};
