//! Kademlia DHT discovery for Borgkit — pure Zig, UDP/JSON transport.
//!
//! This is NOT libp2p wire-compatible. It uses JSON messages over UDP rather
//! than protobuf, and is designed to interoperate only with other Zig Borgkit
//! nodes running this same implementation.
//!
//! Key derivation matches the Rust `Libp2pDiscovery`:
//!   capability key : SHA-256("borgkit:cap:<capability>") → 64-hex chars
//!   ANR value key  : SHA-256("borgkit:anr:<agentId>")   → 64-hex chars
//!
//! Transport   : UDP datagrams, newline-terminated JSON, max 8 KiB
//! Routing     : XOR metric, k = 20 per bucket, 256-bucket routing table
//! Concurrency : alpha = 3 parallel lookups
//!
//! Usage:
//!
//!   var disc = try Libp2pDiscovery.init(allocator, .{
//!       .identity_bytes = my_32_bytes,
//!       .listen_port    = 6174,
//!       .bootstrap_addrs = &.{"192.168.1.5:6174"},
//!   });
//!   try disc.start();
//!   defer disc.deinit();
//!
//!   try disc.register(entry);
//!
//!   var results = std.ArrayList(types.DiscoveryEntry).init(allocator);
//!   defer results.deinit();
//!   try disc.query("text-generation", &results);

const std = @import("std");
const types = @import("types.zig");
const discovery = @import("discovery.zig");

// ── Protocol constants ────────────────────────────────────────────────────────

const K = 20; // k-bucket capacity
const ALPHA = 3; // lookup concurrency
const MAX_MSG_SIZE = 8192; // max UDP datagram in bytes
const QUERY_TIMEOUT_MS: i64 = 2000; // wait time for DHT queries
const VALUE_TIMEOUT_MS: i64 = 500; // wait time for single GET_VALUE
const MDNS_GROUP = "224.0.0.251";
const MDNS_PORT: u16 = 5380;

// ── Public configuration ──────────────────────────────────────────────────────

pub const NodeId = [32]u8;

pub const NodeInfo = struct {
    id: NodeId,
    /// Heap-allocated "ip:port" string owned by the routing table.
    addr: []const u8,
    last_seen_ms: i64,
};

pub const KadConfig = struct {
    /// 32 identity bytes. Node ID = SHA-256 of this.
    identity_bytes: [32]u8 = [_]u8{0} ** 32,
    /// UDP port to bind. Default 6174.
    listen_port: u16 = 6174,
    /// Bootstrap peers ("ip:port"). Caller owns memory.
    bootstrap_addrs: []const []const u8 = &.{},
    /// Re-announce entries every N seconds. Default 30.
    heartbeat_secs: u64 = 30,
    /// Send LAN multicast announce on startup. Default true.
    enable_mdns: bool = true,
};

// ── DHT envelope (stored as base64(JSON)) ─────────────────────────────────────

/// Wire format stored in the `values` map and transmitted via PUT_VALUE/VALUE.
const DhtEnvelope = struct {
    v: u32,
    seq: u64,
    entry: EntryFields,
    sig: []const u8,
};

/// Flat representation of a DiscoveryEntry for DHT serialization.
const EntryFields = struct {
    agent_id: []const u8,
    name: []const u8,
    owner: []const u8,
    capabilities: []const []const u8,
    protocol: []const u8,
    host: []const u8,
    port: u16,
    tls: bool,
    peer_id: []const u8,
    multiaddr: []const u8,
    status: []const u8,
    registered_at_ms: i64,
};

// ── Pending query tracking ────────────────────────────────────────────────────

const PendingQuery = struct {
    /// Accumulates provider "ip:port" strings (heap-allocated, owned by us).
    result_buf: *std.ArrayList([]const u8),
    /// Receives raw decoded VALUE bytes (heap-allocated, owned by us).
    value_buf: *?[]u8,
    done: *std.atomic.Value(bool),
};

// ── K-bucket ──────────────────────────────────────────────────────────────────

const KBucket = struct {
    nodes: std.ArrayList(NodeInfo),
    allocator: std.mem.Allocator,

    fn init(allocator: std.mem.Allocator) KBucket {
        return .{
            .nodes = std.ArrayList(NodeInfo).init(allocator),
            .allocator = allocator,
        };
    }

    fn deinit(self: *KBucket) void {
        for (self.nodes.items) |n| self.allocator.free(n.addr);
        self.nodes.deinit();
    }

    /// Add or refresh a node. Evicts the oldest entry when the bucket is full.
    fn update(self: *KBucket, node: NodeInfo) !void {
        // Refresh existing entry in place.
        for (self.nodes.items) |*n| {
            if (std.mem.eql(u8, &n.id, &node.id)) {
                self.allocator.free(n.addr);
                n.addr = try self.allocator.dupe(u8, node.addr);
                n.last_seen_ms = node.last_seen_ms;
                return;
            }
        }
        if (self.nodes.items.len < K) {
            try self.nodes.append(.{
                .id = node.id,
                .addr = try self.allocator.dupe(u8, node.addr),
                .last_seen_ms = node.last_seen_ms,
            });
            return;
        }
        // Evict index 0 (oldest / head).
        self.allocator.free(self.nodes.items[0].addr);
        self.nodes.items[0] = .{
            .id = node.id,
            .addr = try self.allocator.dupe(u8, node.addr),
            .last_seen_ms = node.last_seen_ms,
        };
    }

    fn remove(self: *KBucket, id: NodeId) void {
        var i: usize = 0;
        while (i < self.nodes.items.len) {
            if (std.mem.eql(u8, &self.nodes.items[i].id, &id)) {
                self.allocator.free(self.nodes.items[i].addr);
                _ = self.nodes.swapRemove(i);
            } else {
                i += 1;
            }
        }
    }
};

// ── Routing table ─────────────────────────────────────────────────────────────

const RoutingTable = struct {
    local_id: NodeId,
    buckets: [256]KBucket,
    mutex: std.Thread.Mutex,

    fn init(allocator: std.mem.Allocator, local_id: NodeId) !RoutingTable {
        var rt: RoutingTable = .{
            .local_id = local_id,
            .buckets = undefined,
            .mutex = .{},
        };
        for (&rt.buckets) |*b| b.* = KBucket.init(allocator);
        return rt;
    }

    fn deinit(self: *RoutingTable) void {
        for (&self.buckets) |*b| b.deinit();
    }

    fn update(self: *RoutingTable, node: NodeInfo) !void {
        // Never store ourselves.
        if (std.mem.eql(u8, &node.id, &self.local_id)) return;
        const idx = bucketIdx(self.local_id, node.id);
        self.mutex.lock();
        defer self.mutex.unlock();
        try self.buckets[idx].update(node);
    }

    fn remove(self: *RoutingTable, id: NodeId) void {
        const idx = bucketIdx(self.local_id, id);
        self.mutex.lock();
        defer self.mutex.unlock();
        self.buckets[idx].remove(id);
    }

    /// Return up to n nodes closest to target (XOR distance).
    /// The returned slice is owned by the caller (allocated with `allocator`),
    /// but the addr strings inside still point into the routing table — do not
    /// free them; copy them first if you need them to outlive the table lock.
    fn closest(self: *RoutingTable, target: NodeId, n: usize, allocator: std.mem.Allocator) ![]NodeInfo {
        self.mutex.lock();
        defer self.mutex.unlock();

        var all = std.ArrayList(NodeInfo).init(allocator);
        defer all.deinit();
        for (&self.buckets) |*b| {
            for (b.nodes.items) |node| try all.append(node);
        }

        const Ctx = struct {
            target: NodeId,
            pub fn lessThan(ctx: @This(), a: NodeInfo, b_node: NodeInfo) bool {
                for (ctx.target, 0..) |tb, i| {
                    const da = a.id[i] ^ tb;
                    const db = b_node.id[i] ^ tb;
                    if (da != db) return da < db;
                }
                return false;
            }
        };
        std.mem.sort(NodeInfo, all.items, Ctx{ .target = target }, Ctx.lessThan);

        const take = @min(n, all.items.len);
        return allocator.dupe(NodeInfo, all.items[0..take]);
    }

    /// Bucket index = position of the highest differing bit.
    /// Walk bytes left-to-right; for each byte find the highest set bit in
    /// `local[i] ^ id[i]`.  Returns 255 for identical IDs.
    fn bucketIdx(local: NodeId, id: NodeId) u8 {
        for (local, 0..) |lb, i| {
            const xor = lb ^ id[i];
            if (xor == 0) continue;
            const high_bit: u8 = 7 - @clz(xor); // 0..7
            return @intCast(i * 8 + (7 - high_bit));
        }
        return 255;
    }
};

// ── Main struct ───────────────────────────────────────────────────────────────

pub const Libp2pDiscovery = struct {
    allocator: std.mem.Allocator,
    config: KadConfig,
    local_id: NodeId,
    routing: RoutingTable,
    socket: std.posix.fd_t,
    local_addr: []const u8, // "0.0.0.0:<port>"

    // In-memory stores — all protected by entries_mutex.
    entries_mutex: std.Thread.Mutex,
    local_entries: std.StringHashMap(DhtEnvelope), // agentId → envelope
    providers: std.StringHashMap(std.ArrayList([]const u8)), // hex(key) → addrs
    values: std.StringHashMap([]u8), // hex(key) → raw decoded bytes

    // Pending queries — protected by pending_mutex.
    pending_mutex: std.Thread.Mutex,
    pending: std.StringHashMap(PendingQuery),

    // Background threads.
    recv_thread: ?std.Thread,
    hb_thread: ?std.Thread,
    running: std.atomic.Value(bool),
    seq: std.atomic.Value(u64),

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Bind the UDP socket. Does not start background threads.
    pub fn init(allocator: std.mem.Allocator, config: KadConfig) !Libp2pDiscovery {
        var local_id: NodeId = undefined;
        std.crypto.hash.sha2.Sha256.hash(&config.identity_bytes, &local_id, .{});

        const sock = try std.posix.socket(
            std.posix.AF.INET,
            std.posix.SOCK.DGRAM,
            std.posix.IPPROTO.UDP,
        );
        errdefer std.posix.close(sock);

        // Allow rapid restart without TIME_WAIT blocking the port.
        const one: u32 = 1;
        try std.posix.setsockopt(
            sock,
            std.posix.SOL.SOCKET,
            std.posix.SO.REUSEADDR,
            std.mem.asBytes(&one),
        );

        const bind_addr = try std.net.Address.parseIp4("0.0.0.0", config.listen_port);
        try std.posix.bind(sock, &bind_addr.any, bind_addr.getOsSockLen());

        const local_addr = try std.fmt.allocPrint(allocator, "0.0.0.0:{d}", .{config.listen_port});
        errdefer allocator.free(local_addr);

        return .{
            .allocator = allocator,
            .config = config,
            .local_id = local_id,
            .routing = try RoutingTable.init(allocator, local_id),
            .socket = sock,
            .local_addr = local_addr,
            .entries_mutex = .{},
            .local_entries = std.StringHashMap(DhtEnvelope).init(allocator),
            .providers = std.StringHashMap(std.ArrayList([]const u8)).init(allocator),
            .values = std.StringHashMap([]u8).init(allocator),
            .pending_mutex = .{},
            .pending = std.StringHashMap(PendingQuery).init(allocator),
            .recv_thread = null,
            .hb_thread = null,
            .running = std.atomic.Value(bool).init(false),
            .seq = std.atomic.Value(u64).init(0),
        };
    }

    /// Start the receive loop, heartbeat loop, ping bootstrap peers, and
    /// optionally send an mDNS multicast announce.
    pub fn start(self: *Libp2pDiscovery) !void {
        self.running.store(true, .release);

        self.recv_thread = try std.Thread.spawn(.{}, recvLoop, .{self});
        self.hb_thread = try std.Thread.spawn(.{}, heartbeatLoop, .{self});

        for (self.config.bootstrap_addrs) |peer| {
            self.sendPing(peer) catch |e| {
                std.log.warn("[Kad] bootstrap ping {s}: {}", .{ peer, e });
            };
        }

        if (self.config.enable_mdns) {
            self.sendMdnsAnnounce() catch |e| {
                std.log.warn("[Kad] mDNS announce: {}", .{e});
            };
        }

        std.log.info("[Kad] started on {s}, node_id={s}", .{
            self.local_addr,
            idToHex(self.local_id)[0..],
        });
    }

    /// Stop all background threads and free every resource.
    pub fn deinit(self: *Libp2pDiscovery) void {
        self.running.store(false, .release);
        // Closing the socket unblocks any pending recvfrom.
        std.posix.close(self.socket);

        if (self.recv_thread) |t| t.join();
        if (self.hb_thread) |t| t.join();

        self.routing.deinit();
        self.allocator.free(self.local_addr);

        // Free local_entries (DhtEnvelope does not own its slices here;
        // they point into the original DiscoveryEntry passed to register()).
        {
            var it = self.local_entries.keyIterator();
            while (it.next()) |k| self.allocator.free(k.*);
            self.local_entries.deinit();
        }

        // Free providers.
        {
            var it = self.providers.iterator();
            while (it.next()) |kv| {
                self.allocator.free(kv.key_ptr.*);
                for (kv.value_ptr.items) |a| self.allocator.free(a);
                kv.value_ptr.deinit();
            }
            self.providers.deinit();
        }

        // Free values.
        {
            var it = self.values.iterator();
            while (it.next()) |kv| {
                self.allocator.free(kv.key_ptr.*);
                self.allocator.free(kv.value_ptr.*);
            }
            self.values.deinit();
        }

        self.pending.deinit();
    }

    // ── IAgentDiscovery interface ─────────────────────────────────────────────

    /// Store locally and announce to the DHT (ADD_PROVIDER + PUT_VALUE).
    pub fn register(self: *Libp2pDiscovery, entry: types.DiscoveryEntry) !void {
        const seq = self.seq.fetchAdd(1, .acq_rel);

        const env = DhtEnvelope{
            .v = 1,
            .seq = seq,
            .entry = .{
                .agent_id = entry.agent_id,
                .name = entry.name,
                .owner = entry.owner,
                .capabilities = entry.capabilities,
                .protocol = @tagName(entry.network.protocol),
                .host = entry.network.host,
                .port = entry.network.port,
                .tls = entry.network.tls,
                .peer_id = entry.network.peer_id,
                .multiaddr = entry.network.multiaddr,
                .status = @tagName(entry.health),
                .registered_at_ms = entry.registered_at,
            },
            .sig = "",
        };

        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            const key = try self.allocator.dupe(u8, entry.agent_id);
            errdefer self.allocator.free(key);
            // Replace any previous entry for the same agent_id.
            if (self.local_entries.fetchRemove(entry.agent_id)) |old| {
                self.allocator.free(old.key);
            }
            try self.local_entries.put(key, env);
        }

        try self.announceEntry(entry, seq);
        std.log.info("[Kad] registered agent {s}", .{entry.agent_id});
    }

    /// Remove from the local store. DHT records at remote nodes expire naturally.
    pub fn unregister(self: *Libp2pDiscovery, agent_id: []const u8) void {
        self.entries_mutex.lock();
        defer self.entries_mutex.unlock();
        if (self.local_entries.fetchRemove(agent_id)) |kv| {
            self.allocator.free(kv.key);
        }
        std.log.info("[Kad] unregistered agent {s}", .{agent_id});
    }

    /// Query DHT for capability providers; appends matching entries to `out`.
    /// Sends GET_PROVIDERS to the closest known nodes and waits up to 2 s.
    pub fn query(
        self: *Libp2pDiscovery,
        capability: []const u8,
        out: *std.ArrayList(types.DiscoveryEntry),
    ) !void {
        // 1. Derive capability key.
        const cap_prefix = try std.fmt.allocPrint(self.allocator, "borgkit:cap:{s}", .{capability});
        defer self.allocator.free(cap_prefix);
        const cap_key_hex = sha256Hex(cap_prefix);

        // 2. Set up pending query (providers phase).
        const prov_nonce = nonceHex();
        var result_buf = std.ArrayList([]const u8).init(self.allocator);
        var dummy_val: ?[]u8 = null;
        var prov_done = std.atomic.Value(bool).init(false);

        {
            self.pending_mutex.lock();
            defer self.pending_mutex.unlock();
            const k = try self.allocator.dupe(u8, &prov_nonce);
            errdefer self.allocator.free(k);
            try self.pending.put(k, .{
                .result_buf = &result_buf,
                .value_buf = &dummy_val,
                .done = &prov_done,
            });
        }
        defer {
            self.pending_mutex.lock();
            if (self.pending.fetchRemove(&prov_nonce)) |kv| self.allocator.free(kv.key);
            self.pending_mutex.unlock();
            for (result_buf.items) |a| self.allocator.free(a);
            result_buf.deinit();
            if (dummy_val) |v| self.allocator.free(v);
        }

        // 3. Send GET_PROVIDERS to closest nodes.
        {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const nodes = try self.routing.closest(self.local_id, ALPHA, arena.allocator());
            for (nodes) |node| {
                const msg = try std.fmt.allocPrint(arena.allocator(),
                    "{{\"type\":\"get_providers\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"key\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                    .{ idToHex(self.local_id), self.local_addr, cap_key_hex, prov_nonce },
                );
                self.sendTo(node.addr, msg) catch {};
            }
        }

        // 4. Wait for PROVIDERS response.
        const prov_deadline = std.time.milliTimestamp() + QUERY_TIMEOUT_MS;
        while (!prov_done.load(.acquire) and std.time.milliTimestamp() < prov_deadline) {
            std.time.sleep(std.time.ns_per_ms * 10);
        }

        // 5. Fetch full entries from each provider via GET_VALUE.
        const val_nonce = nonceHex();
        var val_buf: ?[]u8 = null;
        var val_done = std.atomic.Value(bool).init(false);
        var dummy_list = std.ArrayList([]const u8).init(self.allocator);
        defer {
            for (dummy_list.items) |a| self.allocator.free(a);
            dummy_list.deinit();
            if (val_buf) |v| self.allocator.free(v);
        }

        {
            self.pending_mutex.lock();
            defer self.pending_mutex.unlock();
            const k = try self.allocator.dupe(u8, &val_nonce);
            errdefer self.allocator.free(k);
            try self.pending.put(k, .{
                .result_buf = &dummy_list,
                .value_buf = &val_buf,
                .done = &val_done,
            });
        }
        defer {
            self.pending_mutex.lock();
            if (self.pending.fetchRemove(&val_nonce)) |kv| self.allocator.free(kv.key);
            self.pending_mutex.unlock();
        }

        var arena2 = std.heap.ArenaAllocator.init(self.allocator);
        defer arena2.deinit();

        for (result_buf.items) |provider_addr| {
            val_done.store(false, .release);
            const msg = try std.fmt.allocPrint(arena2.allocator(),
                "{{\"type\":\"get_value\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"key\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                .{ idToHex(self.local_id), self.local_addr, cap_key_hex, val_nonce },
            );
            self.sendTo(provider_addr, msg) catch continue;

            const val_deadline = std.time.milliTimestamp() + VALUE_TIMEOUT_MS;
            while (!val_done.load(.acquire) and std.time.milliTimestamp() < val_deadline) {
                std.time.sleep(std.time.ns_per_ms * 10);
            }
            if (val_buf) |vb| {
                const entry = decodeDhtValue(vb, arena2.allocator()) catch {
                    self.allocator.free(vb);
                    val_buf = null;
                    continue;
                };
                if (capabilityMatches(entry, capability)) {
                    out.append(try discovery.cloneDiscoveryEntry(self.allocator, entry)) catch {};
                }
                self.allocator.free(vb);
                val_buf = null;
            }
        }

        // 6. Include matching local_entries.
        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            var it = self.local_entries.iterator();
            while (it.next()) |kv| {
                const ef = kv.value_ptr.entry;
                for (ef.capabilities) |cap| {
                    if (std.mem.eql(u8, cap, capability)) {
                        const de = envelopeToDiscoveryEntry(ef, arena2.allocator()) catch continue;
                        out.append(discovery.cloneDiscoveryEntry(self.allocator, de) catch continue) catch {};
                        break;
                    }
                }
            }
        }
    }

    /// Look up a specific agent by ID from the DHT value store.
    pub fn findById(self: *Libp2pDiscovery, agent_id: []const u8) !?types.DiscoveryEntry {
        // Fast path: check local entries.
        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            if (self.local_entries.get(agent_id)) |env| {
                var arena = std.heap.ArenaAllocator.init(self.allocator);
                defer arena.deinit();
                const de = try envelopeToDiscoveryEntry(env.entry, arena.allocator());
                return try discovery.cloneDiscoveryEntry(self.allocator, de);
            }
        }

        // Derive ANR key and check local value store.
        const anr_prefix = try std.fmt.allocPrint(self.allocator, "borgkit:anr:{s}", .{agent_id});
        defer self.allocator.free(anr_prefix);
        const key_hex = sha256Hex(anr_prefix);

        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            if (self.values.get(&key_hex)) |vb| {
                var arena = std.heap.ArenaAllocator.init(self.allocator);
                defer arena.deinit();
                const de = decodeDhtValue(vb, arena.allocator()) catch return null;
                return try discovery.cloneDiscoveryEntry(self.allocator, de);
            }
        }

        // Issue GET_VALUE to the DHT.
        const nonce_hex = nonceHex();
        var val_buf: ?[]u8 = null;
        var val_done = std.atomic.Value(bool).init(false);
        var dummy_list = std.ArrayList([]const u8).init(self.allocator);
        defer {
            for (dummy_list.items) |a| self.allocator.free(a);
            dummy_list.deinit();
            if (val_buf) |v| self.allocator.free(v);
        }

        {
            self.pending_mutex.lock();
            defer self.pending_mutex.unlock();
            const k = try self.allocator.dupe(u8, &nonce_hex);
            errdefer self.allocator.free(k);
            try self.pending.put(k, .{
                .result_buf = &dummy_list,
                .value_buf = &val_buf,
                .done = &val_done,
            });
        }
        defer {
            self.pending_mutex.lock();
            if (self.pending.fetchRemove(&nonce_hex)) |kv| self.allocator.free(kv.key);
            self.pending_mutex.unlock();
        }

        {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const target = hexToId(&key_hex) catch self.local_id;
            const nodes = try self.routing.closest(target, ALPHA, arena.allocator());
            for (nodes) |node| {
                const msg = try std.fmt.allocPrint(arena.allocator(),
                    "{{\"type\":\"get_value\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"key\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                    .{ idToHex(self.local_id), self.local_addr, key_hex, nonce_hex },
                );
                self.sendTo(node.addr, msg) catch {};
            }
        }

        const deadline = std.time.milliTimestamp() + QUERY_TIMEOUT_MS;
        while (!val_done.load(.acquire) and std.time.milliTimestamp() < deadline) {
            std.time.sleep(std.time.ns_per_ms * 10);
        }

        if (val_buf) |vb| {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const de = decodeDhtValue(vb, arena.allocator()) catch return null;
            return try discovery.cloneDiscoveryEntry(self.allocator, de);
        }
        return null;
    }

    /// Mark the agent healthy and re-announce to the DHT.
    pub fn heartbeat(self: *Libp2pDiscovery, agent_id: []const u8) void {
        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            if (self.local_entries.getPtr(agent_id)) |env| {
                env.entry.status = "healthy";
            } else {
                return;
            }
        }
        std.log.info("[Kad] heartbeat for {s}", .{agent_id});
    }

    // ── Background loops ──────────────────────────────────────────────────────

    fn recvLoop(self: *Libp2pDiscovery) void {
        var buf: [MAX_MSG_SIZE]u8 = undefined;
        while (self.running.load(.acquire)) {
            var src: std.posix.sockaddr = undefined;
            var src_len: std.posix.socklen_t = @sizeOf(std.posix.sockaddr);
            const n = std.posix.recvfrom(self.socket, &buf, 0, &src, &src_len) catch |e| {
                if (self.running.load(.acquire)) {
                    std.log.warn("[Kad] recvfrom: {}", .{e});
                }
                continue;
            };
            const msg = buf[0..n];

            // Format the source address as "ip:port".
            const addr_net = std.net.Address{ .any = src };
            var addr_buf: [64]u8 = undefined;
            const addr_str = std.fmt.bufPrint(&addr_buf, "{}", .{addr_net}) catch continue;

            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            self.handleMessage(msg, addr_str, arena.allocator()) catch |e| {
                std.log.warn("[Kad] handleMessage: {}", .{e});
            };
        }
    }

    fn heartbeatLoop(self: *Libp2pDiscovery) void {
        const interval_ns = self.config.heartbeat_secs * std.time.ns_per_s;
        while (self.running.load(.acquire)) {
            std.time.sleep(interval_ns);
            if (!self.running.load(.acquire)) break;

            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();

            // Snapshot entries to avoid holding the mutex during network I/O.
            self.entries_mutex.lock();
            var envs = std.ArrayList(DhtEnvelope).init(arena.allocator());
            var it = self.local_entries.iterator();
            while (it.next()) |kv| envs.append(kv.value_ptr.*) catch {};
            self.entries_mutex.unlock();

            for (envs.items) |env| {
                const de = envelopeToDiscoveryEntry(env.entry, arena.allocator()) catch continue;
                self.announceEntry(de, env.seq) catch |e| {
                    std.log.warn("[Kad] heartbeat announce: {}", .{e});
                };
            }
        }
    }

    // ── Message handler ───────────────────────────────────────────────────────

    fn handleMessage(
        self: *Libp2pDiscovery,
        msg: []const u8,
        from_addr: []const u8,
        arena: std.mem.Allocator,
    ) !void {
        const parsed = try std.json.parseFromSlice(std.json.Value, arena, msg, .{});
        const obj = switch (parsed.value) {
            .object => |m| m,
            else => return error.InvalidMessage,
        };

        const msg_type = switch (obj.get("type") orelse return error.InvalidMessage) {
            .string => |s| s,
            else => return error.InvalidMessage,
        };
        const from_id_hex = switch (obj.get("from_id") orelse return error.InvalidMessage) {
            .string => |s| s,
            else => return error.InvalidMessage,
        };
        const nonce: []const u8 = if (obj.get("nonce")) |nv|
            switch (nv) {
                .string => |s| s,
                else => "",
            }
        else
            "";

        // Update routing table with the sender.
        const from_id = try hexToId(from_id_hex);
        try self.routing.update(.{
            .id = from_id,
            .addr = from_addr,
            .last_seen_ms = std.time.milliTimestamp(),
        });

        if (std.mem.eql(u8, msg_type, "ping")) {
            // ── PING → send PONG ──────────────────────────────────────────────
            const pong = try std.fmt.allocPrint(arena,
                "{{\"type\":\"pong\",\"from_id\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                .{ idToHex(self.local_id), nonce },
            );
            try self.sendTo(from_addr, pong);

        } else if (std.mem.eql(u8, msg_type, "pong")) {
            // ── PONG → routing table already updated above ────────────────────

        } else if (std.mem.eql(u8, msg_type, "announce")) {
            // ── mDNS ANNOUNCE → ping back ─────────────────────────────────────
            self.sendPing(from_addr) catch {};

        } else if (std.mem.eql(u8, msg_type, "find_node")) {
            // ── FIND_NODE → return K closest nodes to target ──────────────────
            const target_hex = switch (obj.get("target") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            const target = try hexToId(target_hex);
            const nodes = try self.routing.closest(target, K, arena);
            const resp = try buildNodesJson(nodes, self.local_id, nonce, arena);
            try self.sendTo(from_addr, resp);

        } else if (std.mem.eql(u8, msg_type, "nodes")) {
            // ── NODES → add nodes to routing table, signal pending ────────────
            const nodes_val = obj.get("nodes") orelse return;
            const nodes_arr = switch (nodes_val) {
                .array => |a| a.items,
                else => return,
            };
            for (nodes_arr) |item| {
                const no = switch (item) {
                    .object => |m| m,
                    else => continue,
                };
                const id_hex = switch (no.get("id") orelse continue) {
                    .string => |s| s,
                    else => continue,
                };
                const addr = switch (no.get("addr") orelse continue) {
                    .string => |s| s,
                    else => continue,
                };
                const nid = hexToId(id_hex) catch continue;
                self.routing.update(.{
                    .id = nid,
                    .addr = addr,
                    .last_seen_ms = std.time.milliTimestamp(),
                }) catch {};
            }
            self.signalPendingDone(nonce);

        } else if (std.mem.eql(u8, msg_type, "add_provider")) {
            // ── ADD_PROVIDER → store sender as provider for key ───────────────
            const key_hex = switch (obj.get("key") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            const gop = try self.providers.getOrPut(key_hex);
            if (!gop.found_existing) {
                // We must own the key.
                gop.key_ptr.* = try self.allocator.dupe(u8, key_hex);
                gop.value_ptr.* = std.ArrayList([]const u8).init(self.allocator);
            }
            // Add addr only once.
            var already = false;
            for (gop.value_ptr.items) |a| {
                if (std.mem.eql(u8, a, from_addr)) {
                    already = true;
                    break;
                }
            }
            if (!already) {
                try gop.value_ptr.append(try self.allocator.dupe(u8, from_addr));
            }

        } else if (std.mem.eql(u8, msg_type, "get_providers")) {
            // ── GET_PROVIDERS → return providers + closer nodes ───────────────
            const key_hex = switch (obj.get("key") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            // Snapshot the provider list under the lock.
            var provs_snap = std.ArrayList([]const u8).init(arena);
            {
                self.entries_mutex.lock();
                defer self.entries_mutex.unlock();
                if (self.providers.get(key_hex)) |pl| {
                    for (pl.items) |a| provs_snap.append(a) catch {};
                }
            }
            const target = hexToId(key_hex) catch self.local_id;
            const closer = try self.routing.closest(target, K, arena);
            const resp = try buildProvidersJson(
                provs_snap.items,
                closer,
                self.local_id,
                key_hex,
                nonce,
                arena,
            );
            try self.sendTo(from_addr, resp);

        } else if (std.mem.eql(u8, msg_type, "providers")) {
            // ── PROVIDERS → update routing + signal pending query ─────────────
            if (obj.get("closer_nodes")) |cn_val| {
                const cn_arr = switch (cn_val) {
                    .array => |a| a.items,
                    else => &[_]std.json.Value{},
                };
                for (cn_arr) |item| {
                    const no = switch (item) {
                        .object => |m| m,
                        else => continue,
                    };
                    const id_hex = switch (no.get("id") orelse continue) {
                        .string => |s| s,
                        else => continue,
                    };
                    const addr = switch (no.get("addr") orelse continue) {
                        .string => |s| s,
                        else => continue,
                    };
                    const nid = hexToId(id_hex) catch continue;
                    self.routing.update(.{
                        .id = nid,
                        .addr = addr,
                        .last_seen_ms = std.time.milliTimestamp(),
                    }) catch {};
                }
            }
            const provs_val = obj.get("providers") orelse return;
            const provs_arr = switch (provs_val) {
                .array => |a| a.items,
                else => return,
            };
            {
                self.pending_mutex.lock();
                defer self.pending_mutex.unlock();
                if (self.pending.get(nonce)) |pq| {
                    for (provs_arr) |item| {
                        const no = switch (item) {
                            .object => |m| m,
                            else => continue,
                        };
                        const addr = switch (no.get("addr") orelse continue) {
                            .string => |s| s,
                            else => continue,
                        };
                        const owned = self.allocator.dupe(u8, addr) catch continue;
                        pq.result_buf.append(owned) catch {
                            self.allocator.free(owned);
                        };
                    }
                    pq.done.store(true, .release);
                }
            }

        } else if (std.mem.eql(u8, msg_type, "put_value")) {
            // ── PUT_VALUE → store decoded bytes ───────────────────────────────
            const key_hex = switch (obj.get("key") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            const value_b64 = switch (obj.get("value") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            const dec_len = std.base64.standard.Decoder.calcSizeForSlice(value_b64) catch return;
            const decoded = try self.allocator.alloc(u8, dec_len);
            errdefer self.allocator.free(decoded);
            std.base64.standard.Decoder.decode(decoded, value_b64) catch {
                self.allocator.free(decoded);
                return;
            };
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            const gop = try self.values.getOrPut(key_hex);
            if (gop.found_existing) {
                self.allocator.free(gop.value_ptr.*);
            } else {
                gop.key_ptr.* = try self.allocator.dupe(u8, key_hex);
            }
            gop.value_ptr.* = decoded;

        } else if (std.mem.eql(u8, msg_type, "get_value")) {
            // ── GET_VALUE → send VALUE if we have it, else NODES ─────────────
            const key_hex = switch (obj.get("key") orelse return error.InvalidMessage) {
                .string => |s| s,
                else => return error.InvalidMessage,
            };
            var found_val: ?[]u8 = null;
            {
                self.entries_mutex.lock();
                defer self.entries_mutex.unlock();
                found_val = self.values.get(key_hex);
            }
            if (found_val) |vb| {
                const enc_len = std.base64.standard.Encoder.calcSize(vb.len);
                const encoded = try arena.alloc(u8, enc_len);
                _ = std.base64.standard.Encoder.encode(encoded, vb);
                const resp = try std.fmt.allocPrint(arena,
                    "{{\"type\":\"value\",\"from_id\":\"{s}\",\"key\":\"{s}\",\"value\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                    .{ idToHex(self.local_id), key_hex, encoded, nonce },
                );
                try self.sendTo(from_addr, resp);
            } else {
                // Return closer nodes so the requester can continue iterating.
                const target = hexToId(key_hex) catch self.local_id;
                const nodes = try self.routing.closest(target, K, arena);
                const resp = try buildNodesJson(nodes, self.local_id, nonce, arena);
                try self.sendTo(from_addr, resp);
            }

        } else if (std.mem.eql(u8, msg_type, "value")) {
            // ── VALUE → deliver decoded bytes to pending query ────────────────
            const value_b64 = switch (obj.get("value") orelse return) {
                .string => |s| s,
                else => return,
            };
            const dec_len = std.base64.standard.Decoder.calcSizeForSlice(value_b64) catch return;
            const decoded = try self.allocator.alloc(u8, dec_len);
            errdefer self.allocator.free(decoded);
            std.base64.standard.Decoder.decode(decoded, value_b64) catch {
                self.allocator.free(decoded);
                return;
            };
            {
                self.pending_mutex.lock();
                defer self.pending_mutex.unlock();
                if (self.pending.get(nonce)) |pq| {
                    if (pq.value_buf.*) |old| self.allocator.free(old);
                    pq.value_buf.* = decoded;
                    pq.done.store(true, .release);
                } else {
                    self.allocator.free(decoded);
                }
            }
        }
    }

    // ── DHT announce ──────────────────────────────────────────────────────────

    fn announceEntry(self: *Libp2pDiscovery, entry: types.DiscoveryEntry, seq: u64) !void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();

        const value_b64 = try encodeDhtValue(entry, seq, arena.allocator());

        const anr_prefix = try std.fmt.allocPrint(arena.allocator(), "borgkit:anr:{s}", .{entry.agent_id});
        const anr_key_hex = sha256Hex(anr_prefix);

        // Also store locally so findById() works before any peer replies.
        {
            self.entries_mutex.lock();
            defer self.entries_mutex.unlock();
            const dec_len = std.base64.standard.Decoder.calcSizeForSlice(value_b64) catch 0;
            if (dec_len > 0) {
                const decoded = try self.allocator.alloc(u8, dec_len);
                std.base64.standard.Decoder.decode(decoded, value_b64) catch {
                    self.allocator.free(decoded);
                };
                const gop = try self.values.getOrPut(&anr_key_hex);
                if (gop.found_existing) {
                    self.allocator.free(gop.value_ptr.*);
                } else {
                    gop.key_ptr.* = try self.allocator.dupe(u8, &anr_key_hex);
                }
                gop.value_ptr.* = decoded;
            }
        }

        const nodes = try self.routing.closest(self.local_id, ALPHA, arena.allocator());

        for (entry.capabilities) |cap| {
            const cap_prefix = try std.fmt.allocPrint(arena.allocator(), "borgkit:cap:{s}", .{cap});
            const cap_key_hex = sha256Hex(cap_prefix);

            // Also register ourselves as a provider locally.
            {
                self.entries_mutex.lock();
                defer self.entries_mutex.unlock();
                const gop = try self.providers.getOrPut(&cap_key_hex);
                if (!gop.found_existing) {
                    gop.key_ptr.* = try self.allocator.dupe(u8, &cap_key_hex);
                    gop.value_ptr.* = std.ArrayList([]const u8).init(self.allocator);
                }
                var already = false;
                for (gop.value_ptr.items) |a| {
                    if (std.mem.eql(u8, a, self.local_addr)) {
                        already = true;
                        break;
                    }
                }
                if (!already) {
                    try gop.value_ptr.append(try self.allocator.dupe(u8, self.local_addr));
                }
            }

            for (nodes) |node| {
                const ap = try std.fmt.allocPrint(arena.allocator(),
                    "{{\"type\":\"add_provider\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"key\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                    .{ idToHex(self.local_id), self.local_addr, cap_key_hex, nonceHex() },
                );
                self.sendTo(node.addr, ap) catch {};

                const pv = try std.fmt.allocPrint(arena.allocator(),
                    "{{\"type\":\"put_value\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"key\":\"{s}\",\"value\":\"{s}\",\"nonce\":\"{s}\"}}\n",
                    .{ idToHex(self.local_id), self.local_addr, anr_key_hex, value_b64, nonceHex() },
                );
                self.sendTo(node.addr, pv) catch {};
            }
        }
    }

    // ── Network ───────────────────────────────────────────────────────────────

    fn sendPing(self: *Libp2pDiscovery, addr: []const u8) !void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const msg = try std.fmt.allocPrint(arena.allocator(),
            "{{\"type\":\"ping\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\",\"nonce\":\"{s}\"}}\n",
            .{ idToHex(self.local_id), self.local_addr, nonceHex() },
        );
        try self.sendTo(addr, msg);
    }

    fn sendMdnsAnnounce(self: *Libp2pDiscovery) !void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const msg = try std.fmt.allocPrint(arena.allocator(),
            "{{\"type\":\"announce\",\"from_id\":\"{s}\",\"from_addr\":\"{s}\"}}\n",
            .{ idToHex(self.local_id), self.local_addr },
        );
        const dest = try std.fmt.allocPrint(arena.allocator(), "{s}:{d}", .{ MDNS_GROUP, MDNS_PORT });
        self.sendTo(dest, msg) catch |e| {
            std.log.warn("[Kad] mDNS send: {}", .{e});
        };
    }

    /// Parse "ip:port" (or "hostname:port"), resolve, and send a UDP datagram.
    fn sendTo(self: *Libp2pDiscovery, addr_str: []const u8, msg: []const u8) !void {
        const colon = std.mem.lastIndexOfScalar(u8, addr_str, ':') orelse return error.BadAddress;
        const host = addr_str[0..colon];
        const port = try std.fmt.parseInt(u16, addr_str[colon + 1 ..], 10);
        const dest = try std.net.Address.parseIp(host, port);
        _ = try std.posix.sendto(self.socket, msg, 0, &dest.any, dest.getOsSockLen());
    }

    fn signalPendingDone(self: *Libp2pDiscovery, nonce: []const u8) void {
        self.pending_mutex.lock();
        defer self.pending_mutex.unlock();
        if (self.pending.get(nonce)) |pq| {
            pq.done.store(true, .release);
        }
    }
};

// ── Serialization helpers ─────────────────────────────────────────────────────

/// Build the DHT envelope JSON and return it base64-encoded (arena-allocated).
fn encodeDhtValue(
    entry: types.DiscoveryEntry,
    seq: u64,
    arena: std.mem.Allocator,
) ![]const u8 {
    var json_buf = std.ArrayList(u8).init(arena);
    const w = json_buf.writer();

    try w.print("{{\"v\":1,\"seq\":{d},\"entry\":{{", .{seq});
    try w.writeAll("\"agent_id\":\"");
    try writeEscaped(w, entry.agent_id);
    try w.writeAll("\",\"name\":\"");
    try writeEscaped(w, entry.name);
    try w.writeAll("\",\"owner\":\"");
    try writeEscaped(w, entry.owner);
    try w.writeAll("\",\"capabilities\":[");
    for (entry.capabilities, 0..) |cap, i| {
        if (i > 0) try w.writeByte(',');
        try w.writeByte('"');
        try writeEscaped(w, cap);
        try w.writeByte('"');
    }
    try w.writeAll("],\"protocol\":\"");
    try w.writeAll(@tagName(entry.network.protocol));
    try w.writeAll("\",\"host\":\"");
    try writeEscaped(w, entry.network.host);
    try w.print("\",\"port\":{d},\"tls\":{s},\"peer_id\":\"", .{
        entry.network.port,
        if (entry.network.tls) "true" else "false",
    });
    try writeEscaped(w, entry.network.peer_id);
    try w.writeAll("\",\"multiaddr\":\"");
    try writeEscaped(w, entry.network.multiaddr);
    try w.writeAll("\",\"status\":\"");
    try w.writeAll(@tagName(entry.health));
    try w.print("\",\"registered_at_ms\":{d}", .{entry.registered_at});
    try w.writeAll("},\"sig\":\"\"}");

    const json_bytes = json_buf.items;
    const enc_len = std.base64.standard.Encoder.calcSize(json_bytes.len);
    const encoded = try arena.alloc(u8, enc_len);
    _ = std.base64.standard.Encoder.encode(encoded, json_bytes);
    return encoded;
}

/// Decode a base64-encoded DHT envelope into a DiscoveryEntry (arena-allocated).
fn decodeDhtValue(value_b64: []const u8, arena: std.mem.Allocator) !types.DiscoveryEntry {
    const dec_len = try std.base64.standard.Decoder.calcSizeForSlice(value_b64);
    const decoded = try arena.alloc(u8, dec_len);
    try std.base64.standard.Decoder.decode(decoded, value_b64);

    const parsed = try std.json.parseFromSlice(std.json.Value, arena, decoded, .{});
    const root = switch (parsed.value) {
        .object => |m| m,
        else => return error.InvalidDhtValue,
    };
    const entry_val = root.get("entry") orelse return error.InvalidDhtValue;
    const ef = switch (entry_val) {
        .object => |m| m,
        else => return error.InvalidDhtValue,
    };

    const agent_id = try getStr(arena, ef, "agent_id");
    const name = try getStr(arena, ef, "name");
    const owner = try getStr(arena, ef, "owner");
    const host = try getStr(arena, ef, "host");
    const peer_id = try getStrOr(arena, ef, "peer_id", "");
    const multiaddr = try getStrOr(arena, ef, "multiaddr", "");
    const status = try getStrOr(arena, ef, "status", "healthy");
    const proto_s = try getStrOr(arena, ef, "protocol", "http");

    const caps_val = ef.get("capabilities") orelse return error.InvalidDhtValue;
    const caps_arr = switch (caps_val) {
        .array => |a| a.items,
        else => return error.InvalidDhtValue,
    };
    const caps = try arena.alloc([]const u8, caps_arr.len);
    for (caps_arr, 0..) |cv, i| {
        caps[i] = switch (cv) {
            .string => |s| try arena.dupe(u8, s),
            else => return error.InvalidDhtValue,
        };
    }

    const port: u16 = blk: {
        const pv = ef.get("port") orelse break :blk 6174;
        break :blk switch (pv) {
            .integer => |iv| @intCast(std.math.clamp(iv, 0, 65535)),
            else => 6174,
        };
    };
    const tls: bool = blk: {
        const tv = ef.get("tls") orelse break :blk false;
        break :blk switch (tv) {
            .bool => |b| b,
            else => false,
        };
    };
    const reg_at: i64 = blk: {
        const rv = ef.get("registered_at_ms") orelse break :blk 0;
        break :blk switch (rv) {
            .integer => |iv| iv,
            else => 0,
        };
    };

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
        .health = healthFromString(status),
        .registered_at = reg_at,
    };
}

/// Convert EntryFields (from a DhtEnvelope) to a DiscoveryEntry using `arena`.
fn envelopeToDiscoveryEntry(ef: EntryFields, arena: std.mem.Allocator) !types.DiscoveryEntry {
    const caps = try arena.alloc([]const u8, ef.capabilities.len);
    for (ef.capabilities, 0..) |c, i| caps[i] = try arena.dupe(u8, c);
    return .{
        .agent_id = try arena.dupe(u8, ef.agent_id),
        .name = try arena.dupe(u8, ef.name),
        .owner = try arena.dupe(u8, ef.owner),
        .capabilities = caps,
        .network = .{
            .protocol = protocolFromString(ef.protocol),
            .host = try arena.dupe(u8, ef.host),
            .port = ef.port,
            .tls = ef.tls,
            .peer_id = try arena.dupe(u8, ef.peer_id),
            .multiaddr = try arena.dupe(u8, ef.multiaddr),
        },
        .health = healthFromString(ef.status),
        .registered_at = ef.registered_at_ms,
    };
}

// ── Message builders ──────────────────────────────────────────────────────────

fn buildNodesJson(
    nodes: []NodeInfo,
    local_id: NodeId,
    nonce: []const u8,
    arena: std.mem.Allocator,
) ![]const u8 {
    var buf = std.ArrayList(u8).init(arena);
    const w = buf.writer();
    try w.print("{{\"type\":\"nodes\",\"from_id\":\"{s}\",\"nodes\":[", .{idToHex(local_id)});
    for (nodes, 0..) |n, i| {
        if (i > 0) try w.writeByte(',');
        try w.print("{{\"id\":\"{s}\",\"addr\":\"{s}\"}}", .{ idToHex(n.id), n.addr });
    }
    try w.print("],\"nonce\":\"{s}\"}}\n", .{nonce});
    return buf.items;
}

fn buildProvidersJson(
    providers: [][]const u8,
    closer: []NodeInfo,
    local_id: NodeId,
    key_hex: []const u8,
    nonce: []const u8,
    arena: std.mem.Allocator,
) ![]const u8 {
    var buf = std.ArrayList(u8).init(arena);
    const w = buf.writer();
    try w.print(
        "{{\"type\":\"providers\",\"from_id\":\"{s}\",\"key\":\"{s}\",\"providers\":[",
        .{ idToHex(local_id), key_hex },
    );
    for (providers, 0..) |addr, i| {
        if (i > 0) try w.writeByte(',');
        try w.print("{{\"addr\":\"{s}\"}}", .{addr});
    }
    try w.writeAll("],\"closer_nodes\":[");
    for (closer, 0..) |n, i| {
        if (i > 0) try w.writeByte(',');
        try w.print("{{\"id\":\"{s}\",\"addr\":\"{s}\"}}", .{ idToHex(n.id), n.addr });
    }
    try w.print("],\"nonce\":\"{s}\"}}\n", .{nonce});
    return buf.items;
}

// ── Crypto / encoding ─────────────────────────────────────────────────────────

/// SHA-256(input) → 64-char lower-hex string.
fn sha256Hex(input: []const u8) [64]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(input, &digest, .{});
    return idToHex(digest);
}

/// Parse a 64-hex-char string into a [32]u8 node ID.
fn hexToId(hex: []const u8) ![32]u8 {
    if (hex.len != 64) return error.InvalidHexLength;
    var out: [32]u8 = undefined;
    _ = try std.fmt.hexToBytes(&out, hex);
    return out;
}

/// Encode a [32]u8 node ID as a 64-char lower-hex string.
fn idToHex(id: [32]u8) [64]u8 {
    var out: [64]u8 = undefined;
    const s = std.fmt.bytesToHex(id, .lower);
    @memcpy(&out, &s);
    return out;
}

/// Generate a random 16-byte nonce.
fn genNonce() [16]u8 {
    var n: [16]u8 = undefined;
    std.crypto.random.bytes(&n);
    return n;
}

/// Generate a nonce and return it as a 32-char lower-hex array.
fn nonceHex() [32]u8 {
    const n = genNonce();
    var out: [32]u8 = undefined;
    const s = std.fmt.bytesToHex(n, .lower);
    @memcpy(&out, &s);
    return out;
}

// ── JSON field helpers ────────────────────────────────────────────────────────

fn getStr(arena: std.mem.Allocator, obj: std.json.ObjectMap, key: []const u8) ![]const u8 {
    return switch (obj.get(key) orelse return error.MissingField) {
        .string => |s| try arena.dupe(u8, s),
        else => error.InvalidField,
    };
}

fn getStrOr(
    arena: std.mem.Allocator,
    obj: std.json.ObjectMap,
    key: []const u8,
    default: []const u8,
) ![]const u8 {
    const v = obj.get(key) orelse return arena.dupe(u8, default);
    return switch (v) {
        .string => |s| try arena.dupe(u8, s),
        .null => try arena.dupe(u8, default),
        else => try arena.dupe(u8, default),
    };
}

// ── Enum conversions ──────────────────────────────────────────────────────────

fn protocolFromString(s: []const u8) types.NetworkProtocol {
    if (std.mem.eql(u8, s, "websocket")) return .websocket;
    if (std.mem.eql(u8, s, "grpc")) return .grpc;
    if (std.mem.eql(u8, s, "tcp")) return .tcp;
    return .http;
}

fn healthFromString(s: []const u8) types.HealthStatus {
    if (std.mem.eql(u8, s, "degraded")) return .degraded;
    if (std.mem.eql(u8, s, "unhealthy")) return .unhealthy;
    return .healthy;
}

fn capabilityMatches(entry: types.DiscoveryEntry, capability: []const u8) bool {
    for (entry.capabilities) |cap| {
        if (std.mem.eql(u8, cap, capability)) return true;
    }
    return false;
}

// ── String writing ────────────────────────────────────────────────────────────

fn writeEscaped(writer: anytype, s: []const u8) !void {
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
