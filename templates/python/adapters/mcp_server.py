"""
Borgkit ↔ MCP Bridge — Python (outbound direction)
──────────────────────────────────────────────────────────────────────────────
Exposes any Borgkit agent (IAgent) as an MCP server.

Every Borgkit capability becomes an MCP tool.
Once running, clients like Claude Desktop, Cursor, and any other MCP host
can connect to this agent and call its capabilities as if they were native
MCP tools — with no knowledge of the Borgkit protocol required.

Transports supported
────────────────────
  • stdio  (default) — connect via MCP host config (Claude Desktop, Cursor…)
  • sse              — HTTP Server-Sent Events on a configurable port
  • http             — Streamable HTTP (MCP spec 2025-03-26+)

Usage — stdio (Claude Desktop / Cursor)
───────────────────────────────────────
  import asyncio
  from adapters.mcp_server import serve_as_mcp

  asyncio.run(serve_as_mcp(my_borgkit_agent))

Add to Claude Desktop config  (~/.config/claude/claude_desktop_config.json):
  {
    "mcpServers": {
      "my-agent": {
        "command": "python",
        "args": ["path/to/run_mcp.py"]
      }
    }
  }

Usage — SSE / HTTP (for remote / multi-client scenarios)
─────────────────────────────────────────────────────────
  asyncio.run(serve_as_mcp(agent, transport="sse", port=3000))
  # clients connect to http://localhost:3000/sse
"""

from __future__ import annotations

import asyncio
import uuid
from typing import TYPE_CHECKING, Literal

if TYPE_CHECKING:
    from interfaces.iagent import IAgent


TransportMode = Literal["stdio", "sse", "http"]


async def serve_as_mcp(
    agent:     "IAgent",
    *,
    name:      str | None      = None,
    transport: TransportMode   = "stdio",
    host:      str             = "0.0.0.0",
    port:      int             = 3000,
) -> None:
    """
    Expose *agent* as an MCP server.

    Args:
        agent:     Any Borgkit IAgent instance.
        name:      MCP server name (defaults to agent.agent_id).
        transport: One of "stdio", "sse", "http".
                   Use "stdio" for Claude Desktop / Cursor integrations.
                   Use "sse" or "http" for remote / programmatic clients.
        host:      Bind address for SSE / HTTP transports.
        port:      TCP port for SSE / HTTP transports.

    The function blocks until the server shuts down (stdio closes or
    Ctrl-C is received for network transports).
    """
    try:
        from mcp.server.lowlevel import Server
        from mcp.server.models import InitializationOptions
        import mcp.types as types
    except ImportError:
        raise ImportError(
            "serve_as_mcp() requires the 'mcp' package.\n"
            "Install it: pip install mcp"
        )

    server_name    = name or agent.agent_id
    server_version = _agent_version(agent)

    app = Server(server_name)

    # ── tool list handler ──────────────────────────────────────────────────────

    @app.list_tools()
    async def _list_tools() -> list[types.Tool]:
        return [
            types.Tool(
                name        = cap,
                description = _cap_description(agent, cap),
                inputSchema = _cap_schema(agent, cap),
            )
            for cap in agent.get_capabilities()
            if not cap.startswith("__")        # hide reserved mesh capabilities
        ]

    # ── tool call handler ──────────────────────────────────────────────────────

    @app.call_tool()
    async def _call_tool(
        tool_name: str,
        arguments: dict,
    ) -> list[types.TextContent | types.ImageContent | types.EmbeddedResource]:
        from interfaces.agent_request import AgentRequest

        req = AgentRequest(
            request_id = str(uuid.uuid4()),
            from_id    = "mcp_client",
            capability = tool_name,
            # Accept both wrapped {"payload": {...}} and flat {"key": value} forms
            payload    = arguments.get("payload", arguments) if arguments else {},
        )

        resp = await agent.handle_request(req)

        if resp.status == "success":
            import json
            text = json.dumps(resp.result, ensure_ascii=False, indent=2) if resp.result else "OK"
        elif resp.status == "payment_required":
            text = (
                f"Payment required to call '{tool_name}'.\n"
                f"Requirements: {resp.payment_requirements}"
            )
        else:
            text = f"Error: {resp.error_message}"

        return [types.TextContent(type="text", text=text)]

    # ── start transport ────────────────────────────────────────────────────────

    init_options = InitializationOptions(
        server_name    = server_name,
        server_version = server_version,
        capabilities   = app.get_capabilities(
            notification_options      = None,
            experimental_capabilities = {},
        ),
    )

    if transport == "stdio":
        await _run_stdio(app, init_options)
    elif transport == "sse":
        await _run_sse(app, init_options, host, port, server_name)
    elif transport == "http":
        await _run_http(app, init_options, host, port, server_name)
    else:
        raise ValueError(f"Unknown transport: {transport!r}. Use 'stdio', 'sse', or 'http'.")


# ── transport runners ──────────────────────────────────────────────────────────

async def _run_stdio(app, init_options) -> None:
    import mcp.server.stdio as mcp_stdio
    async with mcp_stdio.stdio_server() as (read, write):
        await app.run(read, write, init_options)


async def _run_sse(app, init_options, host: str, port: int, name: str) -> None:
    try:
        from mcp.server.sse import SseServerTransport
        from aiohttp import web
    except ImportError:
        raise ImportError(
            "SSE transport requires aiohttp: pip install aiohttp"
        )

    sse = SseServerTransport("/messages/")

    async def _handle_sse(request):
        async with sse.connect_sse(request.transport, request.transport.write) as (r, w):
            await app.run(r, w, init_options)
        return web.Response()

    aio_app = web.Application()
    aio_app.router.add_get("/sse",       sse.handle_sse)
    aio_app.router.add_post("/messages/", sse.handle_post_message)

    print(f"[Borgkit→MCP] SSE server '{name}' listening on http://{host}:{port}/sse")
    await web._run_app(aio_app, host=host, port=port)


async def _run_http(app, init_options, host: str, port: int, name: str) -> None:
    try:
        from mcp.server.streamable_http import StreamableHTTPServerTransport
        from aiohttp import web
    except ImportError:
        raise ImportError(
            "Streamable HTTP transport requires aiohttp + mcp>=1.4: pip install aiohttp 'mcp>=1.4'"
        )

    transport = StreamableHTTPServerTransport(mcp_session_id=None)

    async def _handle(request):
        async with transport.connect() as (r, w):
            await app.run(r, w, init_options)
        return web.Response()

    aio_app = web.Application()
    aio_app.router.add_route("*", "/mcp", _handle)

    print(f"[Borgkit→MCP] HTTP server '{name}' listening on http://{host}:{port}/mcp")
    await web._run_app(aio_app, host=host, port=port)


# ── helpers ────────────────────────────────────────────────────────────────────

def _agent_version(agent: "IAgent") -> str:
    meta = getattr(agent, "metadata", None)
    if isinstance(meta, dict):
        return meta.get("version", "0.1.0")
    return "0.1.0"


def _cap_description(agent: "IAgent", cap: str) -> str:
    """
    Return the best available description for a capability.

    Checks (in order):
      1. Plugin CapabilityDescriptor.description  (if agent is a WrappedAgent)
      2. ANR capability description               (if agent.get_anr() has it)
      3. Generic fallback string
    """
    try:
        plugin = getattr(agent, "_plugin", None)
        caps   = getattr(agent, "_caps", {})
        if caps and cap in caps:
            return caps[cap].description or f"Borgkit capability: {cap}"
    except Exception:
        pass
    return f"Borgkit capability: {cap}"


def _cap_schema(agent: "IAgent", cap: str) -> dict:
    """
    Return a JSON Schema for the capability's input.

    Checks CapabilityDescriptor.input_schema first; falls back to a
    generic { payload: object } schema that always works.
    """
    try:
        caps = getattr(agent, "_caps", {})
        if caps and cap in caps:
            schema = caps[cap].input_schema
            if schema:
                return schema
    except Exception:
        pass

    return {
        "type":       "object",
        "properties": {
            "payload": {
                "type":        "object",
                "description": f"Input payload for the '{cap}' capability.",
            }
        },
    }
