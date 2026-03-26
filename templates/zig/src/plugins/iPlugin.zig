//! Sentrix comptime plugin interface.
//!
//! A plugin wraps a "foreign" agent (one that does not natively speak the
//! Sentrix AgentRequest/AgentResponse wire format) and makes it callable via
//! the standard IAgent pipeline.
//!
//! Required declarations on TPlugin:
//!
//!   extractCapabilities(self, agent: *TAgent, allocator: Allocator)
//!       []CapabilityDescriptor
//!       — inspect the foreign agent and return its capabilities.
//!
//!   translateRequest(self, req: AgentRequest, allocator: Allocator)
//!       ![]const u8
//!       — convert a Sentrix AgentRequest to the foreign agent's native input
//!         format (returned as a JSON string or arbitrary bytes).
//!
//!   translateResponse(self, request_id: []const u8, native_output: []const u8)
//!       AgentResponse
//!       — convert the foreign agent's native output back to an AgentResponse.
//!
//!   invokeNative(self, agent: *TAgent, input: []const u8, allocator: Allocator)
//!       ![]const u8
//!       — actually run the foreign agent with the translated input and return
//!         its raw output.
//!
//! Usage:
//!   const Iface = iPlugin.IPlugin(MyPlugin, MyForeignAgent);
//!   const resp  = try Iface.invoke(&my_plugin, &my_agent, req, allocator);

const std = @import("std");
const types = @import("../types.zig");

// ── CapabilityDescriptor ──────────────────────────────────────────────────────

/// Describes a single capability exposed by a wrapped foreign agent.
///
/// All slice fields are borrowed — the plugin is responsible for their lifetime.
pub const CapabilityDescriptor = struct {
    name: []const u8,
    description: []const u8,
    /// JSON Schema string describing the expected input, or null if unknown.
    input_schema: ?[]const u8 = null,
    /// JSON Schema string describing the produced output, or null if unknown.
    output_schema: ?[]const u8 = null,
    /// Human-readable price per call, e.g. "0.001 USDC", or null if free.
    price_per_call: ?[]const u8 = null,
};

// ── IPlugin ───────────────────────────────────────────────────────────────────

/// Comptime-validated plugin interface for wrapping third-party agents.
///
/// Returns a struct with a single `invoke` function that executes the full
/// plugin pipeline:
///   translateRequest → invokeNative → translateResponse
///
/// The comptime block enforces the four required declarations at compile time
/// so that missing methods produce a clear compile error rather than a
/// confusing runtime panic.
pub fn IPlugin(comptime TPlugin: type, comptime TAgent: type) type {
    return struct {
        // ── compile-time interface validation ─────────────────────────────────
        comptime {
            if (!@hasDecl(TPlugin, "extractCapabilities")) {
                @compileError(std.fmt.comptimePrint(
                    "{s} must implement extractCapabilities(self, agent: *{s}, allocator: Allocator) []CapabilityDescriptor",
                    .{ @typeName(TPlugin), @typeName(TAgent) },
                ));
            }
            if (!@hasDecl(TPlugin, "translateRequest")) {
                @compileError(std.fmt.comptimePrint(
                    "{s} must implement translateRequest(self, req: AgentRequest, allocator: Allocator) ![]const u8",
                    .{@typeName(TPlugin)},
                ));
            }
            if (!@hasDecl(TPlugin, "translateResponse")) {
                @compileError(std.fmt.comptimePrint(
                    "{s} must implement translateResponse(self, request_id: []const u8, native_output: []const u8) AgentResponse",
                    .{@typeName(TPlugin)},
                ));
            }
            if (!@hasDecl(TPlugin, "invokeNative")) {
                @compileError(std.fmt.comptimePrint(
                    "{s} must implement invokeNative(self, agent: *{s}, input: []const u8, allocator: Allocator) ![]const u8",
                    .{ @typeName(TPlugin), @typeName(TAgent) },
                ));
            }
        }

        // ── pipeline ──────────────────────────────────────────────────────────

        /// Execute the full plugin pipeline for a single request:
        ///   1. translateRequest  — Sentrix AgentRequest → foreign format
        ///   2. invokeNative      — run the wrapped agent
        ///   3. translateResponse — foreign output → Sentrix AgentResponse
        pub fn invoke(
            plugin: *TPlugin,
            agent: *TAgent,
            req: types.AgentRequest,
            allocator: std.mem.Allocator,
        ) !types.AgentResponse {
            const native_input = try plugin.translateRequest(req, allocator);
            const native_output = try plugin.invokeNative(agent, native_input, allocator);
            return plugin.translateResponse(req.request_id, native_output);
        }

        /// Return the capability descriptors for `agent` as seen through this plugin.
        ///
        /// Delegates directly to TPlugin.extractCapabilities.
        pub fn capabilities(
            plugin: *TPlugin,
            agent: *TAgent,
            allocator: std.mem.Allocator,
        ) []CapabilityDescriptor {
            return plugin.extractCapabilities(agent, allocator);
        }
    };
}
