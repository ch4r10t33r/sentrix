"""
Borgkit ↔ MCP Bridge — Python (inbound direction)
──────────────────────────────────────────────────────────────────────────────
Wraps any MCP-compatible server as a Borgkit agent.

Every MCP tool exposed by the server becomes a Borgkit capability.
The wrapped agent can then be registered with discovery, called by other
Borgkit agents, and served over HTTP with the built-in server — all
without changing a single line of the underlying MCP server.

Supported transports
────────────────────
  • Stdio   — MCP server launched as a subprocess (most common)
  • SSE     — MCP server reachable over HTTP Server-Sent Events
  • HTTP    — Streamable HTTP transport (MCP spec 2025-03-26+)

Usage
─────
  import asyncio
  from plugins.mcp_plugin import MCPPlugin
  from plugins.base import PluginConfig

  config = PluginConfig(
      agent_id="borgkit://agent/github-mcp",
      name="GitHubMCP",
      owner="0xYourWallet",
      port=8081,
  )

  # Connect to an MCP server running as a subprocess
  plugin = await MCPPlugin.from_command(
      command=["npx", "-y", "@modelcontextprotocol/server-github"],
      config=config,
      env={"GITHUB_TOKEN": "ghp_..."},
  )

  # Connect to an MCP server over SSE
  plugin = await MCPPlugin.from_url(
      url="http://localhost:3000/sse",
      config=config,
  )

  agent = plugin.wrap()            # returns a WrappedAgent / IAgent
  asyncio.run(agent.serve(port=8081))
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from typing import Any, Optional

from plugins.base import (
    CapabilityDescriptor,
    PluginConfig,
    BorgkitPlugin,
    WrappedAgent,
)
from interfaces import AgentRequest, AgentResponse


# ── internal type alias ────────────────────────────────────────────────────────

@dataclass
class _MCPTool:
    """Minimal representation of an MCP Tool object."""
    name:        str
    description: str                   = ""
    input_schema: Optional[dict]       = None


# ── plugin ─────────────────────────────────────────────────────────────────────

class MCPPlugin(BorgkitPlugin[None]):
    """
    Borgkit plugin that wraps any MCP server as a discoverable agent.

    Build instances with the async factory methods — do not call the
    constructor directly (the session must be established asynchronously).

    Methods
    -------
    from_command(command, config, env)   Launch an MCP subprocess and connect.
    from_url(url, config, headers)       Connect to an MCP server over SSE / HTTP.
    wrap()                               Return a WrappedAgent (IAgent-compliant).
    close()                              Disconnect from the MCP server.
    """

    def __init__(self, config: PluginConfig) -> None:
        super().__init__(config)
        self._session:  Any           = None   # mcp.ClientSession
        self._tools:    list[_MCPTool] = []
        self._cm_stack: list          = []     # context managers to close on exit

    # ── factory methods ────────────────────────────────────────────────────────

    @classmethod
    async def from_command(
        cls,
        command: list[str],
        config:  PluginConfig,
        env:     Optional[dict[str, str]] = None,
    ) -> "MCPPlugin":
        """
        Launch *command* as a subprocess MCP server and connect to it.

        Args:
            command: argv list, e.g. ``["npx", "-y", "@modelcontextprotocol/server-github"]``
            config:  Borgkit PluginConfig for the wrapped agent.
            env:     Extra environment variables forwarded to the subprocess.

        Example::

            plugin = await MCPPlugin.from_command(
                ["npx", "-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                config,
            )
        """
        try:
            from mcp import ClientSession
            from mcp.client.stdio import StdioServerParameters, stdio_client
        except ImportError:
            raise ImportError(
                "MCPPlugin requires the 'mcp' package.\n"
                "Install it: pip install mcp"
            )

        import os
        merged_env = {**os.environ, **(env or {})}

        plugin = cls(config)
        params = StdioServerParameters(
            command=command[0],
            args=command[1:],
            env=merged_env,
        )
        cm = stdio_client(params)
        read, write = await cm.__aenter__()
        plugin._cm_stack.append(cm)

        session = ClientSession(read, write)
        await session.__aenter__()
        plugin._cm_stack.append(session)

        await session.initialize()
        plugin._session = session
        await plugin._refresh_tools()
        return plugin

    @classmethod
    async def from_url(
        cls,
        url:     str,
        config:  PluginConfig,
        headers: Optional[dict[str, str]] = None,
    ) -> "MCPPlugin":
        """
        Connect to an MCP server over SSE or Streamable HTTP.

        Args:
            url:     The MCP server URL.
                     SSE endpoint:              ``http://host:port/sse``
                     Streamable HTTP endpoint:  ``http://host:port/mcp``
            config:  Borgkit PluginConfig for the wrapped agent.
            headers: Optional HTTP headers (e.g. for authentication).

        Example::

            plugin = await MCPPlugin.from_url(
                "http://localhost:3000/sse",
                config,
                headers={"Authorization": "Bearer sk-..."},
            )
        """
        try:
            from mcp import ClientSession
        except ImportError:
            raise ImportError(
                "MCPPlugin requires the 'mcp' package.\n"
                "Install it: pip install mcp"
            )

        plugin = cls(config)

        # Try Streamable HTTP first (newer spec), fall back to SSE
        transport_cm = _pick_transport(url, headers or {})
        read, write = await transport_cm.__aenter__()
        plugin._cm_stack.append(transport_cm)

        session = ClientSession(read, write)
        await session.__aenter__()
        plugin._cm_stack.append(session)

        await session.initialize()
        plugin._session = session
        await plugin._refresh_tools()
        return plugin

    # ── lifecycle ──────────────────────────────────────────────────────────────

    async def close(self) -> None:
        """Disconnect from the MCP server and clean up resources."""
        for cm in reversed(self._cm_stack):
            try:
                await cm.__aexit__(None, None, None)
            except Exception:
                pass
        self._cm_stack.clear()
        self._session = None

    async def _refresh_tools(self) -> None:
        """Re-fetch the tool list from the MCP server."""
        if not self._session:
            return
        result = await self._session.list_tools()
        self._tools = [
            _MCPTool(
                name=t.name,
                description=getattr(t, "description", "") or "",
                input_schema=(
                    t.inputSchema if isinstance(getattr(t, "inputSchema", None), dict) else None
                ),
            )
            for t in result.tools
        ]

    # ── BorgkitPlugin contract ─────────────────────────────────────────────────

    def extract_capabilities(self, _agent: None) -> list[CapabilityDescriptor]:
        return [
            CapabilityDescriptor(
                name=t.name,
                description=t.description,
                input_schema=t.input_schema,
                native_name=t.name,
            )
            for t in self._tools
        ]

    def translate_request(self, req: AgentRequest, descriptor: CapabilityDescriptor) -> dict:
        """Pass the payload straight through as MCP tool arguments."""
        return req.payload

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """
        Convert MCP tool output to AgentResponse.

        MCP returns a list of content items (TextContent, ImageContent, etc.).
        Text content is joined; binary content is base64-encoded in the result dict.
        """
        if native_result is None:
            return AgentResponse.success(request_id, {"result": None})

        parts: list[str] = []
        blobs: list[dict] = []

        items = native_result if isinstance(native_result, list) else [native_result]
        for item in items:
            item_type = getattr(item, "type", "text")
            if item_type == "text":
                parts.append(getattr(item, "text", str(item)))
            elif item_type == "image":
                import base64
                blobs.append({
                    "type":     "image",
                    "mimeType": getattr(item, "mimeType", "image/png"),
                    "data":     getattr(item, "data", ""),
                })
            elif item_type == "resource":
                resource = getattr(item, "resource", {})
                parts.append(f"[resource: {getattr(resource, 'uri', str(resource))}]")
            else:
                parts.append(str(item))

        result: dict[str, Any] = {}
        if parts:
            result["text"] = "\n".join(parts)
        if blobs:
            result["blobs"] = blobs
        if not result:
            result["raw"] = str(native_result)

        return AgentResponse.success(request_id, result)

    async def invoke_native(
        self,
        _agent:      None,
        descriptor:  CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """Call the MCP tool and return its raw content list."""
        if not self._session:
            raise RuntimeError("MCPPlugin: session is not connected. Call from_command() or from_url() first.")
        result = await self._session.call_tool(descriptor.native_name, native_input)
        return result.content

    # ── override wrap() so agent=None works cleanly ───────────────────────────

    def wrap(self, _agent: None = None) -> WrappedAgent:   # type: ignore[override]
        """
        Return an IAgent-compliant WrappedAgent backed by this MCP session.

        Usage::

            plugin = await MCPPlugin.from_command(...)
            agent  = plugin.wrap()
            await  agent.serve(port=8081)
        """
        caps = self.extract_capabilities(None)
        return WrappedAgent(agent=None, plugin=self, capabilities=caps, config=self.config)


# ── transport helper ───────────────────────────────────────────────────────────

def _pick_transport(url: str, headers: dict):
    """
    Select the right MCP client transport for the given URL.

    Tries Streamable HTTP (mcp >= 1.4) first; falls back to SSE.
    """
    try:
        # Streamable HTTP — preferred for MCP spec 2025-03-26+
        from mcp.client.streamable_http import streamablehttp_client
        return streamablehttp_client(url, headers=headers)
    except ImportError:
        pass

    try:
        # SSE transport — MCP spec pre-2025-03-26
        from mcp.client.sse import sse_client
        return sse_client(url, headers=headers)
    except ImportError:
        pass

    raise ImportError(
        "MCPPlugin: no HTTP transport found in your 'mcp' installation.\n"
        "Upgrade: pip install --upgrade mcp"
    )
