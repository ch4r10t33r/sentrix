//! WrappedAgent — adapts a foreign agent to the Borgkit IAgent interface.
//!
//! Combines IPlugin's translation pipeline with a DiscoveryEntry so that any
//! third-party model or service can participate in the Borgkit mesh without
//! modification.
//!
//! Usage:
//!   var wrapped = WrappedAgent(MyAgent, MyPlugin).init(
//!       &my_agent, &my_plugin,
//!       .{
//!           .agent_id = "borgkit://agent/my-wrapped",
//!           .owner    = "0xABC",
//!       },
//!       allocator,
//!   );
//!
//!   // wrapped satisfies IAgent(WrappedAgent(MyAgent, MyPlugin))
//!   const resp = wrapped.handleRequest(req);
//!
//! IAgent interface check:
//!   The comptime block at the bottom of WrappedAgent validates all four
//!   required declarations so that a compile error appears here (not in iagent)
//!   if the contract is accidentally broken.

const std = @import("std");
const types = @import("../types.zig");
const iPlugin = @import("iPlugin.zig");
const iagent = @import("../iagent.zig");

// ── WrappedAgent ──────────────────────────────────────────────────────────────

/// Returns a struct type that wraps `TAgent` using `TPlugin` and exposes the
/// four IAgent-required declarations.
pub fn WrappedAgent(comptime TAgent: type, comptime TPlugin: type) type {
    return struct {
        const Self = @This();

        // ── fields ────────────────────────────────────────────────────────────

        agent: *TAgent,
        plugin: *TPlugin,
        config: WrappedAgentConfig,
        allocator: std.mem.Allocator,
        /// Cached capability descriptors populated during init().
        capabilities_cache: []iPlugin.CapabilityDescriptor,

        // ── configuration ─────────────────────────────────────────────────────

        pub const WrappedAgentConfig = struct {
            /// Borgkit agent URI, e.g. "borgkit://agent/my-wrapped"
            agent_id: []const u8,
            /// Wallet address or DID of the agent owner.
            owner: []const u8,
            /// Optional IPFS / HTTPS URI for off-chain metadata.
            metadata_uri: ?[]const u8 = null,
            /// The host this agent will be reachable on (reported in ANR).
            network_host: []const u8 = "localhost",
            /// The port this agent will be reachable on (reported in ANR).
            network_port: u16 = 6174,
        };

        // ── init ──────────────────────────────────────────────────────────────

        /// Construct a WrappedAgent.
        ///
        /// Calls `plugin.extractCapabilities(agent, allocator)` immediately so
        /// that getCapabilities() is O(1) afterwards.
        pub fn init(
            agent: *TAgent,
            plugin: *TPlugin,
            config: WrappedAgentConfig,
            allocator: std.mem.Allocator,
        ) Self {
            const Iface = iPlugin.IPlugin(TPlugin, TAgent);
            const caps = Iface.capabilities(plugin, agent, allocator);
            return Self{
                .agent = agent,
                .plugin = plugin,
                .config = config,
                .allocator = allocator,
                .capabilities_cache = caps,
            };
        }

        // ── IAgent interface ──────────────────────────────────────────────────

        /// Return the agent's Borgkit URI.
        pub fn agentId(self: *const Self) []const u8 {
            return self.config.agent_id;
        }

        /// Return the wallet address / DID of the agent owner.
        pub fn owner(self: *const Self) []const u8 {
            return self.config.owner;
        }

        /// Return a slice of capability name strings.
        ///
        /// Extracts `name` from each CapabilityDescriptor in the cache.
        /// The returned slice is allocated from `self.allocator`; callers that
        /// need it to outlive the WrappedAgent should dupe it.
        pub fn getCapabilities(self: *const Self) []const []const u8 {
            // Build a slice of name strings on the fly.
            // We allocate once and cache in a local; for a long-running server
            // this is called rarely so the small allocation is acceptable.
            const names = self.allocator.alloc([]const u8, self.capabilities_cache.len) catch {
                // Allocation failure: return an empty slice rather than panicking.
                return &[_][]const u8{};
            };
            for (self.capabilities_cache, 0..) |desc, i| {
                names[i] = desc.name;
            }
            return names;
        }

        /// Dispatch a request through the plugin pipeline.
        ///
        /// Errors from the pipeline are caught and returned as an AgentResponse
        /// with status=error so that the server can always send a well-formed reply.
        pub fn handleRequest(self: *Self, req: types.AgentRequest) types.AgentResponse {
            const Iface = iPlugin.IPlugin(TPlugin, TAgent);
            return Iface.invoke(self.plugin, self.agent, req, self.allocator) catch |err| {
                return types.AgentResponse.err(req.request_id, @errorName(err));
            };
        }

        /// Return a DiscoveryEntry built from config + live capabilities.
        ///
        /// Called by IAgent(WrappedAgent).getAnr() if it is declared here.
        pub fn getAnr(self: *const Self) types.DiscoveryEntry {
            return types.DiscoveryEntry{
                .agent_id = self.config.agent_id,
                .name = self.config.agent_id, // use agent_id as name by default
                .owner = self.config.owner,
                .capabilities = self.getCapabilities(),
                .network = types.NetworkInfo{
                    .protocol = .http,
                    .host = self.config.network_host,
                    .port = self.config.network_port,
                    .tls = false,
                    .peer_id = "",
                    .multiaddr = "",
                },
                .health = .healthy,
                .registered_at = std.time.milliTimestamp(),
                .metadata_uri = self.config.metadata_uri,
            };
        }

        // ── compile-time interface validation ─────────────────────────────────
        // Triggers a compile error if Self accidentally breaks the IAgent contract.
        comptime {
            _ = iagent.IAgent(Self);
        }
    };
}
