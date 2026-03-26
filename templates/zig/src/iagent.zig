/// Sentrix agent interface (comptime vtable pattern).
///
/// Required declarations on the implementing type:
///   agentId()        []const u8
///   owner()          []const u8
///   getCapabilities() []const []const u8
///   handleRequest()  AgentResponse
///
/// Optional declarations (auto-detected via @hasDecl):
///   preProcess()     — called before handleRequest
///   postProcess()    — called after handleRequest
///   checkPermission() bool — defaults to true
///   getAnr()         DiscoveryEntry — full ANR record (defaults to minimal entry)
///   getPeerId()      ?[]const u8    — libp2p PeerId (defaults to null)
///
/// Usage:
///   const MyAgent = struct {
///       pub fn agentId(_: *const @This()) []const u8 { return "sentrix://agent/my"; }
///       pub fn owner  (_: *const @This()) []const u8 { return "0xWallet"; }
///       pub fn getCapabilities(_: *const @This()) []const []const u8 { return &.{"myCapability"}; }
///       pub fn handleRequest(self: *@This(), req: types.AgentRequest) types.AgentResponse { ... }
///       // Optional: implement getAnr() and getPeerId() for mesh identity exposure
///   };

const std = @import("std");
const types = @import("types.zig");

pub fn IAgent(comptime T: type) type {
    return struct {
        // Verify the implementing type exposes the required interface at compile time
        comptime {
            if (!@hasDecl(T, "agentId"))        @compileError(std.fmt.comptimePrint("{s} must implement agentId()", .{@typeName(T)}));
            if (!@hasDecl(T, "owner"))           @compileError(std.fmt.comptimePrint("{s} must implement owner()", .{@typeName(T)}));
            if (!@hasDecl(T, "getCapabilities")) @compileError(std.fmt.comptimePrint("{s} must implement getCapabilities()", .{@typeName(T)}));
            if (!@hasDecl(T, "handleRequest"))   @compileError(std.fmt.comptimePrint("{s} must implement handleRequest()", .{@typeName(T)}));
        }

        /// Dispatch a request through optional pre/post hooks.
        pub fn dispatch(self: *T, req: types.AgentRequest) types.AgentResponse {
            if (@hasDecl(T, "preProcess")) self.preProcess(req);
            const response = self.handleRequest(req);
            if (@hasDecl(T, "postProcess")) self.postProcess(response);
            return response;
        }

        /// Check if caller has permission for a capability (default: open).
        pub fn checkPermission(_: *T, _: []const u8, _: []const u8) bool {
            return true;
        }

        /// Return the full ANR (Agent Network Record) for this agent.
        ///
        /// Delegates to the implementing type's `getAnr()` if present.
        /// Falls back to a minimal entry built from `agentId()` / `owner()`.
        pub fn getAnr(self: *T, allocator: std.mem.Allocator) types.DiscoveryEntry {
            if (@hasDecl(T, "getAnr")) return self.getAnr(allocator);
            // Default: minimal entry from identity fields
            return types.DiscoveryEntry{
                .agent_id     = self.agentId(),
                .name         = self.agentId(),
                .owner        = self.owner(),
                .capabilities = self.getCapabilities(),
                .network      = types.NetworkInfo{
                    .protocol = .http,
                    .host     = "localhost",
                    .port     = 6174,
                    .tls      = false,
                },
                .health       = .healthy,
                .registered_at = std.time.milliTimestamp(),
                .metadata_uri  = null,
            };
            _ = allocator; // reserved for future JSON serialisation
        }

        /// Return the libp2p PeerId derived from this agent's secp256k1 ANR key.
        ///
        /// Delegates to the implementing type's `getPeerId()` if present.
        /// Returns null by default (anonymous / no signing key).
        pub fn getPeerId(self: *T) ?[]const u8 {
            if (@hasDecl(T, "getPeerId")) return self.getPeerId();
            return null;
        }
    };
}
