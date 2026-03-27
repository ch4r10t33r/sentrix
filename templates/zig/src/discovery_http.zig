//! HttpDiscovery — REST client for a Borgkit-style central registry.
//!
//! Expected API (same as Rust `discovery_http` template):
//!   POST   /agents              register JSON body
//!   DELETE /agents/{id}         unregister
//!   GET    /agents?cap={cap}    query by capability
//!   GET    /agents              list all
//!   PUT    /agents/{id}/hb      heartbeat
//!
//! Environment:
//!   `BORGKIT_DISCOVERY_URL` — base URL (optional trailing slash stripped)
//!   `BORGKIT_DISCOVERY_KEY` — optional value for `X-Api-Key`
//!
//! Callers must free entries from `query` / `listAll` / `findById` with
//! `discovery.freeDiscoveryEntry` when done.

const std = @import("std");
const json = std.json;
const types = @import("types.zig");
const discovery = @import("discovery.zig");

pub const HttpDiscovery = struct {
    allocator: std.mem.Allocator,
    base_url: []const u8,
    client: std.http.Client,
    api_key: ?[]const u8,

    pub fn init(allocator: std.mem.Allocator, base_url: []const u8) HttpDiscovery {
        return initWithKey(allocator, base_url, null);
    }

    pub fn initWithKey(allocator: std.mem.Allocator, base_url: []const u8, api_key: ?[]const u8) HttpDiscovery {
        const trimmed = std.mem.trimRight(u8, base_url, "/");
        const owned_base = allocator.dupe(u8, trimmed) catch @panic("oom");
        const owned_key = if (api_key) |k| allocator.dupe(u8, k) catch @panic("oom") else null;
        return .{
            .allocator = allocator,
            .base_url = owned_base,
            .client = std.http.Client{ .allocator = allocator },
            .api_key = owned_key,
        };
    }

    pub fn deinit(self: *HttpDiscovery) void {
        self.client.deinit();
        self.allocator.free(self.base_url);
        if (self.api_key) |k| self.allocator.free(k);
    }

    /// Returns null if `BORGKIT_DISCOVERY_URL` is unset.
    pub fn fromEnv(allocator: std.mem.Allocator) ?HttpDiscovery {
        const url = std.process.getEnvVarOwned(allocator, "BORGKIT_DISCOVERY_URL") catch return null;
        defer allocator.free(url);
        const key_owned = std.process.getEnvVarOwned(allocator, "BORGKIT_DISCOVERY_KEY") catch null;
        defer if (key_owned) |k| allocator.free(k);
        return initWithKey(allocator, url, key_owned);
    }

    pub fn register(self: *HttpDiscovery, entry: types.DiscoveryEntry) !void {
        const body = try jsonStringifyEntry(self.allocator, entry);
        defer self.allocator.free(body);
        const resp = try self.request(.POST, "/agents", body);
        defer self.allocator.free(resp);
    }

    pub fn unregister(self: *HttpDiscovery, agent_id: []const u8) !void {
        const enc = try percentEncode(self.allocator, agent_id);
        defer self.allocator.free(enc);
        const path = try std.fmt.allocPrint(self.allocator, "/agents/{s}", .{enc});
        defer self.allocator.free(path);
        const resp = try self.request(.DELETE, path, null);
        defer self.allocator.free(resp);
    }

    pub fn query(
        self: *HttpDiscovery,
        capability: []const u8,
        out: *std.ArrayList(types.DiscoveryEntry),
    ) !void {
        const enc = try percentEncode(self.allocator, capability);
        defer self.allocator.free(enc);
        const path = try std.fmt.allocPrint(self.allocator, "/agents?cap={s}", .{enc});
        defer self.allocator.free(path);
        const resp = try self.request(.GET, path, null);
        defer self.allocator.free(resp);
        try appendEntriesFromJsonArray(self.allocator, resp, out);
    }

    pub fn listAll(self: *HttpDiscovery, out: *std.ArrayList(types.DiscoveryEntry)) !void {
        const resp = try self.request(.GET, "/agents", null);
        defer self.allocator.free(resp);
        try appendEntriesFromJsonArray(self.allocator, resp, out);
    }

    pub fn findById(self: *HttpDiscovery, agent_id: []const u8) !?types.DiscoveryEntry {
        var tmp = std.ArrayList(types.DiscoveryEntry).init(self.allocator);
        defer {
            for (tmp.items) |e| discovery.freeDiscoveryEntry(self.allocator, e);
            tmp.deinit();
        }
        try self.listAll(&tmp);
        for (tmp.items) |e| {
            if (std.mem.eql(u8, e.agent_id, agent_id)) {
                return discovery.cloneDiscoveryEntry(self.allocator, e);
            }
        }
        return null;
    }

    pub fn heartbeat(self: *HttpDiscovery, agent_id: []const u8) !void {
        const enc = try percentEncode(self.allocator, agent_id);
        defer self.allocator.free(enc);
        const path = try std.fmt.allocPrint(self.allocator, "/agents/{s}/hb", .{enc});
        defer self.allocator.free(path);
        const resp = try self.request(.PUT, path, null);
        defer self.allocator.free(resp);
    }

    fn request(self: *HttpDiscovery, method: std.http.Method, path_and_query: []const u8, payload: ?[]const u8) ![]u8 {
        const url = try std.fmt.allocPrint(self.allocator, "{s}{s}", .{ self.base_url, path_and_query });
        defer self.allocator.free(url);

        var hdrs: [3]std.http.Header = undefined;
        var n: usize = 0;
        hdrs[n] = .{ .name = "Accept", .value = "application/json" };
        n += 1;
        if (payload != null) {
            hdrs[n] = .{ .name = "Content-Type", .value = "application/json" };
            n += 1;
        }
        if (self.api_key) |key| {
            hdrs[n] = .{ .name = "X-Api-Key", .value = key };
            n += 1;
        }
        const extra = hdrs[0..n];

        var body = std.ArrayList(u8).init(self.allocator);
        defer body.deinit();

        const fr = try self.client.fetch(.{
            .method = method,
            .location = .{ .url = url },
            .extra_headers = extra,
            .payload = payload,
            .response_storage = .{ .dynamic = &body },
        });

        const code = @intFromEnum(fr.status);
        if (code < 200 or code >= 300) return error.HttpDiscoveryBadStatus;

        return body.toOwnedSlice();
    }
};

fn percentEncode(allocator: std.mem.Allocator, s: []const u8) ![]u8 {
    var list = std.ArrayList(u8).init(allocator);
    errdefer list.deinit();
    for (s) |c| {
        switch (c) {
            'A'...'Z', 'a'...'z', '0'...'9', '-', '_', '.', '~' => try list.append(c),
            else => try std.fmt.format(list.writer(), "%{X:0>2}", .{c}),
        }
    }
    return list.toOwnedSlice();
}

fn jsonStringifyEntry(allocator: std.mem.Allocator, entry: types.DiscoveryEntry) ![]u8 {
    var iso_buf: [48]u8 = undefined;
    const reg_iso = millisToRfc3339Utc(&iso_buf, entry.registered_at);
    var hb_buf: [48]u8 = undefined;
    const hb_iso = millisToRfc3339Utc(&hb_buf, std.time.milliTimestamp());

    var list = std.ArrayList(u8).init(allocator);
    errdefer list.deinit();
    const w = list.writer();

    try w.writeAll(
        \\{"agent_id":"
    );
    try escapeJsonString(w, entry.agent_id);
    try w.writeAll(
        \\","name":"
    );
    try escapeJsonString(w, entry.name);
    try w.writeAll(
        \\","owner":"
    );
    try escapeJsonString(w, entry.owner);
    try w.writeAll(
        \\","capabilities":[
    );
    for (entry.capabilities, 0..) |cap, i| {
        if (i > 0) try w.writeByte(',');
        try w.writeByte('"');
        try escapeJsonString(w, cap);
        try w.writeByte('"');
    }
    try w.writeAll(
        \\,"network":{"protocol":"
    );
    try escapeJsonString(w, @tagName(entry.network.protocol));
    try w.writeAll(
        \\","host":"
    );
    try escapeJsonString(w, entry.network.host);
    try std.fmt.format(w, "\",\"port\":{d},\"tls\":", .{entry.network.port});
    try w.writeAll(if (entry.network.tls) "true" else "false");
    try w.writeAll(
        \\,"peer_id":"
    );
    try escapeJsonString(w, entry.network.peer_id);
    try w.writeAll(
        \\","multiaddr":"
    );
    try escapeJsonString(w, entry.network.multiaddr);
    try w.writeAll(
        \\"},"health":{"status":"
    );
    try escapeJsonString(w, @tagName(entry.health));
    try w.writeAll(
        \\","last_heartbeat":"
    );
    try escapeJsonString(w, hb_iso);
    try w.writeAll(
        \\","uptime_seconds":0},"registered_at":"
    );
    try escapeJsonString(w, reg_iso);
    if (entry.metadata_uri) |uri| {
        try w.writeAll(
            \\","metadata_uri":"
        );
        try escapeJsonString(w, uri);
        try w.writeByte('"');
    } else {
        try w.writeAll(
            \\","metadata_uri":null
        );
    }
    try w.writeByte('}');
    return list.toOwnedSlice();
}

fn escapeJsonString(writer: anytype, s: []const u8) !void {
    for (s) |c| {
        switch (c) {
            '\\' => try writer.writeAll("\\\\"),
            '"' => try writer.writeAll("\\\""),
            '\n' => try writer.writeAll("\\n"),
            '\r' => try writer.writeAll("\\r"),
            '\t' => try writer.writeAll("\\t"),
            else => try writer.writeByte(c),
        }
    }
}

fn millisToRfc3339Utc(buf: *[48]u8, ms: i64) []const u8 {
    const sec: i64 = @divFloor(ms, 1000);
    const u_sec: u64 = @intCast(@max(sec, 0));
    const es = std.time.epoch.EpochSeconds{ .secs = u_sec };
    const epoch_day = es.getEpochDay();
    const year_day = epoch_day.calculateYearDay();
    const month_day = year_day.calculateMonthDay();
    const day_secs = es.getDaySeconds();
    const hh = day_secs.getHoursIntoDay();
    const mm = day_secs.getMinutesIntoHour();
    const ss = day_secs.getSecondsIntoMinute();
    const month_num = @as(u8, @intCast(@intFromEnum(month_day.month) + 1));
    const day_num = @as(u8, @intCast(month_day.day_index + 1));
    return std.fmt.bufPrint(buf, "{d:0>4}-{d:0>2}-{d:0>2}T{d:0>2}:{d:0>2}:{d:0>2}Z", .{
        year_day.year,
        month_num,
        day_num,
        hh,
        mm,
        ss,
    }) catch "1970-01-01T00:00:00Z";
}

fn appendEntriesFromJsonArray(
    allocator: std.mem.Allocator,
    raw: []const u8,
    out: *std.ArrayList(types.DiscoveryEntry),
) !void {
    var parsed = try json.parseFromSlice(json.Value, allocator, raw, .{});
    defer parsed.deinit();
    const root = parsed.value;
    const arr: []json.Value = switch (root) {
        .array => |a| a.items,
        else => return error.InvalidDiscoveryJson,
    };
    for (arr) |item| {
        try out.append(try discoveryEntryFromJsonValue(allocator, item));
    }
}

fn discoveryEntryFromJsonValue(allocator: std.mem.Allocator, v: json.Value) !types.DiscoveryEntry {
    const o = switch (v) {
        .object => |m| m,
        else => return error.InvalidDiscoveryJson,
    };

    const agent_id = try dupFieldString(allocator, o, "agent_id") orelse return error.InvalidDiscoveryJson;
    errdefer allocator.free(agent_id);
    const name = try dupFieldString(allocator, o, "name") orelse return error.InvalidDiscoveryJson;
    errdefer allocator.free(name);
    const owner = try dupFieldString(allocator, o, "owner") orelse return error.InvalidDiscoveryJson;
    errdefer allocator.free(owner);

    const caps_val = o.get("capabilities") orelse return error.InvalidDiscoveryJson;
    const caps_items = switch (caps_val) {
        .array => |a| a.items,
        else => return error.InvalidDiscoveryJson,
    };
    const caps = try allocator.alloc([]const u8, caps_items.len);
    errdefer {
        for (caps) |c| allocator.free(c);
        allocator.free(caps);
    }
    for (caps_items, 0..) |cv, i| {
        caps[i] = switch (cv) {
            .string => |s| try allocator.dupe(u8, s),
            else => return error.InvalidDiscoveryJson,
        };
    }

    const net_val = o.get("network") orelse return error.InvalidDiscoveryJson;
    const net = switch (net_val) {
        .object => |m| m,
        else => return error.InvalidDiscoveryJson,
    };
    const proto_s = try dupFieldString(allocator, net, "protocol") orelse try allocator.dupe(u8, "http");
    defer allocator.free(proto_s);
    const host = try dupFieldString(allocator, net, "host") orelse return error.InvalidDiscoveryJson;
    errdefer allocator.free(host);
    const port: u16 = blk: {
        const pv = net.get("port") orelse break :blk 6174;
        break :blk switch (pv) {
            .integer => |i| @intCast(std.math.clamp(i, 0, 65535)),
            .float => |f| @intCast(std.math.clamp(@as(i64, @intFromFloat(@round(f))), 0, 65535)),
            else => 6174,
        };
    };
    const tls_v = net.get("tls") orelse json.Value{ .bool = false };
    const tls = switch (tls_v) {
        .bool => |b| b,
        else => false,
    };
    const peer_id = try dupFieldString(allocator, net, "peer_id") orelse try allocator.dupe(u8, "");
    errdefer allocator.free(peer_id);
    const multiaddr = try dupFieldString(allocator, net, "multiaddr") orelse try allocator.dupe(u8, "");
    errdefer allocator.free(multiaddr);

    const health_v = o.get("health") orelse json.Value{ .string = "healthy" };
    const health = healthFromJson(health_v);

    const reg_v = o.get("registered_at") orelse json.Value{ .integer = 0 };
    const registered_at = parseRegisteredAtMs(reg_v);

    const meta = try dupOptionalFieldString(allocator, o, "metadata_uri");
    errdefer if (meta) |m| allocator.free(m);

    return .{
        .agent_id = agent_id,
        .name = name,
        .owner = owner,
        .capabilities = caps,
        .network = .{
            .protocol = protocolFromString(proto_s),
            .host = host,
            .port = port,
            .tls = tls,
            .peer_id = peer_id,
            .multiaddr = multiaddr,
        },
        .health = health,
        .registered_at = registered_at,
        .metadata_uri = meta,
    };
}

fn dupFieldString(allocator: std.mem.Allocator, o: json.ObjectMap, key: []const u8) !?[]const u8 {
    const v = o.get(key) orelse return null;
    return switch (v) {
        .string => |s| try allocator.dupe(u8, s),
        else => null,
    };
}

fn dupOptionalFieldString(allocator: std.mem.Allocator, o: json.ObjectMap, key: []const u8) !?[]const u8 {
    const v = o.get(key) orelse return null;
    return switch (v) {
        .string => |s| try allocator.dupe(u8, s),
        .null => null,
        else => null,
    };
}

fn healthFromJson(v: json.Value) types.HealthStatus {
    return switch (v) {
        .string => |s| healthFromStatusString(s),
        .object => |m| blk: {
            const st = m.get("status") orelse break :blk .healthy;
            break :blk switch (st) {
                .string => |s| healthFromStatusString(s),
                else => .healthy,
            };
        },
        else => .healthy,
    };
}

fn healthFromStatusString(s: []const u8) types.HealthStatus {
    if (std.mem.eql(u8, s, "healthy")) return .healthy;
    if (std.mem.eql(u8, s, "degraded")) return .degraded;
    if (std.mem.eql(u8, s, "unhealthy")) return .unhealthy;
    return .healthy;
}

fn protocolFromString(s: []const u8) types.NetworkProtocol {
    if (std.mem.eql(u8, s, "http")) return .http;
    if (std.mem.eql(u8, s, "websocket")) return .websocket;
    if (std.mem.eql(u8, s, "grpc")) return .grpc;
    if (std.mem.eql(u8, s, "tcp")) return .tcp;
    return .http;
}

fn parseRegisteredAtMs(v: json.Value) i64 {
    return switch (v) {
        .integer => |i| i,
        .float => |f| @intFromFloat(@round(f)),
        .string => |s| std.fmt.parseInt(i64, s, 10) catch 0,
        else => 0,
    };
}

/// Shared wire JSON for HTTP POST and libp2p gossip payloads.
pub fn stringifyDiscoveryEntryForWire(allocator: std.mem.Allocator, entry: types.DiscoveryEntry) ![]u8 {
    return jsonStringifyEntry(allocator, entry);
}

pub fn parseDiscoveryEntryFromValue(allocator: std.mem.Allocator, v: json.Value) !types.DiscoveryEntry {
    return discoveryEntryFromJsonValue(allocator, v);
}
