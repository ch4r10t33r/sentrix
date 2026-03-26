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

    const client_mod = b.addModule("client", .{
        .root_source_file = b.path("src/client.zig"),
        .imports = &.{
            .{ .name = "types",     .module = types_mod     },
            .{ .name = "discovery", .module = discovery_mod },
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

    // ── main executable ───────────────────────────────────────────────────────

    const exe = b.addExecutable(.{
        .name             = "sentrix-agent",
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

    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());

    const run_step = b.step("run", "Run the Sentrix agent");
    run_step.dependOn(&run_cmd.step);

    // ── zig build server ──────────────────────────────────────────────────────
    //
    // `zig build server` checks that server.zig compiles cleanly.
    // Because server.zig is a library module (not a main), we compile it as a
    // static library so the build system exercises every code path.

    const server_lib = b.addStaticLibrary(.{
        .name             = "sentrix-server",
        .root_source_file = b.path("src/server.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    server_lib.root_module.addImport("types",  types_mod);
    server_lib.root_module.addImport("iagent", iagent_mod);

    const server_step = b.step("server", "Compile the Sentrix HTTP server module");
    server_step.dependOn(&b.addInstallArtifact(server_lib, .{}).step);

    // ── zig build plugins ─────────────────────────────────────────────────────
    //
    // Compile the plugins modules (iPlugin + wrapped_agent) as a static library
    // so `zig build plugins` validates them independently.

    const plugins_lib = b.addStaticLibrary(.{
        .name             = "sentrix-plugins",
        .root_source_file = b.path("src/plugins/iPlugin.zig"),
        .target           = target,
        .optimize         = optimize,
    });
    plugins_lib.root_module.addImport("types",   types_mod);
    plugins_lib.root_module.addImport("iagent",  iagent_mod);
    plugins_lib.root_module.addImport("iPlugin", iplugin_mod);

    const plugins_step = b.step("plugins", "Compile the Sentrix plugins modules");
    plugins_step.dependOn(&b.addInstallArtifact(plugins_lib, .{}).step);

    // ── unit tests ────────────────────────────────────────────────────────────

    const unit_tests = b.addTest(.{
        .root_source_file = b.path("src/example_agent.zig"),
        .target   = target,
        .optimize = optimize,
    });
    unit_tests.root_module.addImport("types",     types_mod);
    unit_tests.root_module.addImport("iagent",    iagent_mod);
    unit_tests.root_module.addImport("discovery", discovery_mod);
    unit_tests.root_module.addImport("client",    client_mod);

    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&b.addRunArtifact(unit_tests).step);
}
