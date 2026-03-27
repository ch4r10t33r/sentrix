const std = @import("std");

pub fn build(b: *std.Build) void {
    const target   = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // ── shared modules ────────────────────────────────────────────────────────
    //
    // Declaring these as named modules means any executable or test added later
    // can import them with `@import("server")`, `@import("iPlugin")`, etc.
    // without having to repeat the path everywhere.

    const types_mod = b.addModule("types", .{
        .root_source_file = b.path("src/types.zig"),
    });

    const iagent_mod = b.addModule("iagent", .{
        .root_source_file = b.path("src/iagent.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
        },
    });

    const discovery_mod = b.addModule("discovery", .{
        .root_source_file = b.path("src/discovery.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
        },
    });

    const discovery_http_mod = b.addModule("discovery_http", .{
        .root_source_file = b.path("src/discovery_http.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
            .{ .name = "discovery", .module = discovery_mod },
        },
    });

    const discovery_libp2p_mod = b.addModule("discovery_libp2p", .{
        .root_source_file = b.path("src/discovery_libp2p.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
            .{ .name = "discovery", .module = discovery_mod },
            .{ .name = "discovery_http", .module = discovery_http_mod },
        },
    });

    const client_mod = b.addModule("client", .{
        .root_source_file = b.path("src/client.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
            .{ .name = "discovery", .module = discovery_mod },
            .{ .name = "discovery_http", .module = discovery_http_mod },
            .{ .name = "discovery_libp2p", .module = discovery_libp2p_mod },
        },
    });

    // ── plugins modules ───────────────────────────────────────────────────────

    const iplugin_mod = b.addModule("iPlugin", .{
        .root_source_file = b.path("src/plugins/iPlugin.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
        },
    });

    const wrapped_agent_mod = b.addModule("wrapped_agent", .{
        .root_source_file = b.path("src/plugins/wrapped_agent.zig"),
        .imports = &.{
            .{ .name = "types",   .module = types_mod   },
            .{ .name = "iagent",  .module = iagent_mod  },
            .{ .name = "iPlugin", .module = iplugin_mod },
        },
    });

    // ── framework bridge plugin modules ───────────────────────────────────────

    const langgraph_mod = b.addModule("langgraph", .{
        .root_source_file = b.path("src/plugins/langgraph.zig"),
        .imports = &.{
            .{ .name = "types",   .module = types_mod   },
            .{ .name = "iPlugin", .module = iplugin_mod },
        },
    });

    const google_adk_mod = b.addModule("google_adk", .{
        .root_source_file = b.path("src/plugins/google_adk.zig"),
        .imports = &.{
            .{ .name = "types",   .module = types_mod   },
            .{ .name = "iPlugin", .module = iplugin_mod },
        },
    });

    const crewai_mod = b.addModule("crewai", .{
        .root_source_file = b.path("src/plugins/crewai.zig"),
        .imports = &.{
            .{ .name = "types",   .module = types_mod   },
            .{ .name = "iPlugin", .module = iplugin_mod },
        },
    });

    // ── server module ─────────────────────────────────────────────────────────
    //
    // server.zig is not a standalone executable — it exports a single `serve`
    // function that an application's main.zig calls.  Expose it as a module so
    // any executable in this build (or downstream builds via `b.dependency`)
    // can do:  const server = @import("server");
    //          try server.serve(MyAgent, &agent, 6174, allocator);

    const server_mod = b.addModule("server", .{
        .root_source_file = b.path("src/server.zig"),
        .imports = &.{
            .{ .name = "types",  .module = types_mod  },
            .{ .name = "iagent", .module = iagent_mod },
        },
    });

    // ── MCP bridge modules ────────────────────────────────────────────────────
    //
    // mcp_plugin.zig  — inbound bridge: wraps an MCP server as a Borgkit agent
    // mcp_server.zig  — outbound bridge: exposes a Borgkit agent as an MCP server

    const mcp_plugin_mod = b.addModule("mcp_plugin", .{
        .root_source_file = b.path("src/mcp_plugin.zig"),
        .imports = &.{
            .{ .name = "types", .module = types_mod },
        },
    });

    const mcp_server_mod = b.addModule("mcp_server", .{
        .root_source_file = b.path("src/mcp_server.zig"),
        .imports = &.{
            .{ .name = "types",  .module = types_mod  },
            .{ .name = "iagent", .module = iagent_mod },
        },
    });

    // ── main executable ───────────────────────────────────────────────────────

    const exe = b.addExecutable(.{
        .name             = "borgkit-agent",
        .root_source_file = b.path("src/example_agent.zig"),
        .target           = target,
        .optimize         = optimize,
    });

    // Wire all modules into the main executable so example_agent.zig (and any
    // file it imports) can reach them with plain @import("<name>") calls.
    exe.root_module.addImport("types",        types_mod);
    exe.root_module.addImport("iagent",       iagent_mod);
    exe.root_module.addImport("discovery",    discovery_mod);
    exe.root_module.addImport("client",       client_mod);
    exe.root_module.addImport("iPlugin",      iplugin_mod);
    exe.root_module.addImport("wrapped_agent", wrapped_agent_mod);
    exe.root_module.addImport("server",       server_mod);
    exe.root_module.addImport("langgraph",    langgraph_mod);
    exe.root_module.addImport("google_adk",   google_adk_mod);
    exe.root_module.addImport("crewai",       crewai_mod);
    exe.root_module.addImport("mcp_plugin",   mcp_plugin_mod);
    exe.root_module.addImport("mcp_server",   mcp_server_mod);

    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());

    const run_step = b.step("run", "Run the Borgkit agent");
    run_step.dependOn(&run_cmd.step);

    // ── zig build server ──────────────────────────────────────────────────────
    //
    // `zig build server` checks that server.zig compiles cleanly.
    // Because server.zig is a library module (not a main), we compile it as a
    // static library so the build system exercises every code path.

    const server_lib = b.addStaticLibrary(.{
        .name             = "borgkit-server",
        .root_source_file = b.path("src/server.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    server_lib.root_module.addImport("types",  types_mod);
    server_lib.root_module.addImport("iagent", iagent_mod);

    const server_step = b.step("server", "Compile the Borgkit HTTP server module");
    server_step.dependOn(&b.addInstallArtifact(server_lib, .{}).step);

    // ── zig build plugins ─────────────────────────────────────────────────────
    //
    // Compile the plugins modules (iPlugin + wrapped_agent) as a static library
    // so `zig build plugins` validates them independently.

    const plugins_lib = b.addStaticLibrary(.{
        .name             = "borgkit-plugins",
        .root_source_file = b.path("src/plugins/iPlugin.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    plugins_lib.root_module.addImport("types",   types_mod);
    plugins_lib.root_module.addImport("iagent",  iagent_mod);
    plugins_lib.root_module.addImport("iPlugin", iplugin_mod);

    const plugins_step = b.step("plugins", "Compile the Borgkit plugins modules");
    plugins_step.dependOn(&b.addInstallArtifact(plugins_lib, .{}).step);

    // ── zig build mcp ─────────────────────────────────────────────────────────
    //
    // Compile both MCP bridge modules as static libraries so `zig build mcp`
    // validates them independently of the main executable.

    const mcp_plugin_lib = b.addStaticLibrary(.{
        .name             = "borgkit-mcp-plugin",
        .root_source_file = b.path("src/mcp_plugin.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    mcp_plugin_lib.root_module.addImport("types", types_mod);

    const mcp_server_lib = b.addStaticLibrary(.{
        .name             = "borgkit-mcp-server",
        .root_source_file = b.path("src/mcp_server.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    mcp_server_lib.root_module.addImport("types",  types_mod);
    mcp_server_lib.root_module.addImport("iagent", iagent_mod);

    const mcp_step = b.step("mcp", "Compile the MCP bridge modules (mcp_plugin + mcp_server)");
    mcp_step.dependOn(&b.addInstallArtifact(mcp_plugin_lib, .{}).step);
    mcp_step.dependOn(&b.addInstallArtifact(mcp_server_lib, .{}).step);

    // ── example programs: did:key + gossip fan-out ────────────────────────────

    const ex_did = b.addExecutable(.{
        .name             = "example-did-key",
        .root_source_file = b.path("examples/did_key_identity.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    const ex_gossip = b.addExecutable(.{
        .name             = "example-gossip-fanout",
        .root_source_file = b.path("examples/gossip_fanout_discovery.zig"),
        .target           = target,
        .optimize         = optimize,
    });

    const examples_step = b.step("examples", "Build did:key + gossip fan-out demos");
    examples_step.dependOn(&b.addInstallArtifact(ex_did, .{}).step);
    examples_step.dependOn(&b.addInstallArtifact(ex_gossip, .{}).step);

    // ── unit tests ────────────────────────────────────────────────────────────

    const unit_tests = b.addTest(.{
        .root_source_file = b.path("src/example_agent.zig"),
        .target   = target,
        .optimize = optimize,
    });
    unit_tests.root_module.addImport("types",     types_mod);
    unit_tests.root_module.addImport("iagent",    iagent_mod);
    unit_tests.root_module.addImport("discovery", discovery_mod);
    unit_tests.root_module.addImport("client", client_mod);
    unit_tests.root_module.addImport("discovery_http", discovery_http_mod);
    unit_tests.root_module.addImport("discovery_libp2p", discovery_libp2p_mod);

    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&b.addRunArtifact(unit_tests).step);
}
