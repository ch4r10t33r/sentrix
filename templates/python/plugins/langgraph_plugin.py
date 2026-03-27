"""
LangGraph → Borgkit Plugin
──────────────────────────────────────────────────────────────────────────────
Wraps a compiled LangGraph graph (or any callable with a `.invoke()` method)
so it appears as a standard Borgkit IAgent on the mesh.

Capability extraction strategy
──────────────────────────────
LangGraph has no single universal tool registry, so we use four strategies
in priority order:

  1. Explicit map in PluginConfig.capability_map
       { "borgkit_cap_name": "node_or_tool_name" }
  2. Tools bound on the graph's LLM node  (most common ReAct / Tool-Use pattern)
       graph.nodes["agent"].bound.tools  or  graph.nodes["agent"].tools
  3. tools= kwarg passed directly to LangGraphPlugin
  4. Single-capability fallback: one capability = one graph invocation

Usage
─────
  from plugins.langgraph_plugin import LangGraphPlugin, LangGraphPluginConfig
  from plugins.base import PluginConfig

  config = LangGraphPluginConfig(
      agent_id   = "borgkit://agent/weather",
      name       = "WeatherAgent",
      version    = "1.0.0",
      tags       = ["weather", "langraph"],
      # optional: expose each tool as a separate Borgkit capability
      expose_tools_as_capabilities = True,
  )
  plugin = LangGraphPlugin(config)
  agent  = plugin.wrap(compiled_graph)

  await agent.register_discovery()

  # Borgkit will now call agent.handle_request(req)
  # e.g. req.capability == "getWeather", req.payload == {"city": "London"}

Install deps:
  pip install langgraph langchain-core
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Optional

from interfaces import AgentRequest, AgentResponse
from plugins.base import BorgkitPlugin, CapabilityDescriptor, PluginConfig


# ── extended config ───────────────────────────────────────────────────────────

@dataclass
class LangGraphPluginConfig(PluginConfig):
    """LangGraph-specific configuration (extends PluginConfig)."""

    # If True, each tool in the graph is exposed as a separate Borgkit capability.
    # If False, the entire graph is exposed as one capability named `invoke`.
    expose_tools_as_capabilities: bool = True

    # The LangGraph state key that holds the user input (default: "messages")
    input_key: str = "messages"

    # The LangGraph state key where we read the final output (default: "messages")
    output_key: str = "messages"

    # Node name to inspect for bound tools (default: "agent" or "tools")
    agent_node_name: str = "agent"

    # Stream output instead of waiting for full result
    stream: bool = False

    # Recursion limit passed to graph.invoke()
    recursion_limit: int = 25


# ── plugin implementation ─────────────────────────────────────────────────────

class LangGraphPlugin(BorgkitPlugin):
    """
    Borgkit plugin for LangGraph agents.

    Accepts any LangGraph `CompiledGraph` (or duck-typed equivalent).
    """

    def __init__(
        self,
        config: LangGraphPluginConfig,
        tools: Optional[list] = None,
    ):
        super().__init__(config)
        self._lg_config    = config
        self._explicit_tools = tools or []

    # ── capability extraction ─────────────────────────────────────────────────

    def extract_capabilities(self, agent: Any) -> list[CapabilityDescriptor]:
        """
        Walk the graph looking for tools; fall back to a single `invoke` cap.
        """
        if self._lg_config.expose_tools_as_capabilities:
            tools = self._discover_tools(agent)
            if tools:
                return [self._tool_to_descriptor(t) for t in tools]

        # Single-graph capability
        return [CapabilityDescriptor(
            name        = "invoke",
            description = self._lg_config.description or "Invoke the LangGraph agent",
            native_name = "__graph__",
            tags        = self._lg_config.tags,
        )]

    def _discover_tools(self, graph: Any) -> list:
        """
        Try multiple strategies to find tools in a LangGraph graph.
        Returns a list of LangChain BaseTool instances (or duck-types).
        """
        # 1. Explicit tools passed to plugin constructor
        if self._explicit_tools:
            return self._explicit_tools

        # 2. Explicit capability_map → create stub descriptors
        if self.config.capability_map:
            return []  # handled via capability_map in translate_request

        # 3. Inspect graph nodes for bound tools
        try:
            nodes = getattr(graph, 'nodes', {})
            node_name = self._lg_config.agent_node_name
            node = nodes.get(node_name)
            if node:
                # RunnableBinding with tools
                for attr in ('bound', 'runnable'):
                    inner = getattr(node, attr, None)
                    if inner:
                        tools = getattr(inner, 'tools', None)
                        if tools:
                            return list(tools)
                # Direct .tools attribute
                tools = getattr(node, 'tools', None)
                if tools:
                    return list(tools)
        except Exception:
            pass

        # 4. graph.tools attribute (some custom patterns)
        try:
            tools = getattr(graph, 'tools', None)
            if tools:
                return list(tools)
        except Exception:
            pass

        return []

    @staticmethod
    def _tool_to_descriptor(tool: Any) -> CapabilityDescriptor:
        """Convert a LangChain BaseTool to a CapabilityDescriptor."""
        name = getattr(tool, 'name', str(tool))
        desc = getattr(tool, 'description', '')

        # Try to extract JSON schema from tool args_schema (Pydantic model)
        input_schema = None
        try:
            schema_model = getattr(tool, 'args_schema', None)
            if schema_model and hasattr(schema_model, 'schema'):
                input_schema = schema_model.schema()
            elif schema_model and hasattr(schema_model, 'model_json_schema'):
                input_schema = schema_model.model_json_schema()
        except Exception:
            pass

        return CapabilityDescriptor(
            name         = name,
            description  = desc,
            native_name  = name,
            input_schema = input_schema,
            tags         = [],
        )

    # ── request translation ───────────────────────────────────────────────────

    def translate_request(
        self,
        req: AgentRequest,
        descriptor: CapabilityDescriptor,
    ) -> dict:
        """
        Build a LangGraph-compatible state dict from an AgentRequest.

        Strategy:
          - Single graph invocation: wrap payload as a HumanMessage
          - Per-tool invocation: wrap as a ToolCall message
        """
        # Resolve native tool name via capability_map or descriptor
        native = (
            self.config.capability_map.get(req.capability)
            or descriptor.native_name
            or req.capability
        )

        input_key = self._lg_config.input_key

        if native == "__graph__":
            # Whole-graph invocation: treat payload as a human message
            from langchain_core.messages import HumanMessage
            content = (
                req.payload.get("message")
                or req.payload.get("input")
                or json.dumps(req.payload)
            )
            return {
                input_key: [HumanMessage(content=content)],
                "__borgkit_request_id__": req.request_id,
                "__borgkit_capability__": req.capability,
                "__borgkit_from__": req.from_id,
            }
        else:
            # Tool-specific invocation: inject a tool call into the messages
            from langchain_core.messages import HumanMessage
            tool_input = json.dumps(req.payload)
            return {
                input_key: [HumanMessage(content=tool_input)],
                "__borgkit_request_id__": req.request_id,
                "__borgkit_tool__": native,
            }

    # ── response translation ──────────────────────────────────────────────────

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """
        Convert LangGraph's state dict output into a Borgkit AgentResponse.
        Handles both dict states (standard) and BaseMessage list outputs.
        """
        try:
            output_key = self._lg_config.output_key

            # State dict
            if isinstance(native_result, dict):
                messages = native_result.get(output_key, [])
                content  = self._extract_content(messages)
                raw      = native_result
            # Direct message list
            elif isinstance(native_result, list):
                content = self._extract_content(native_result)
                raw     = {"messages": native_result}
            else:
                content = str(native_result)
                raw     = {"raw": str(native_result)}

            return AgentResponse.success(request_id, {
                "content": content,
                "raw":     raw,
            })
        except Exception as e:
            return AgentResponse.error(request_id, f"Response translation failed: {e}")

    @staticmethod
    def _extract_content(messages: list) -> str:
        """Pull text from the last non-tool AI message."""
        if not messages:
            return ""
        # Walk backwards to find the last AIMessage
        for msg in reversed(messages):
            kind = type(msg).__name__
            if kind in ("AIMessage", "AIMessageChunk"):
                content = getattr(msg, 'content', '')
                if isinstance(content, list):   # multimodal
                    return " ".join(
                        c.get("text", "") if isinstance(c, dict) else str(c)
                        for c in content
                    )
                return str(content)
        # Fallback: last message content
        last = messages[-1]
        return str(getattr(last, 'content', last))

    # ── native invocation ─────────────────────────────────────────────────────

    async def invoke_native(
        self,
        agent: Any,
        descriptor: CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """
        Invoke the LangGraph compiled graph.
        Supports both sync (.invoke) and async (.ainvoke) graphs.
        """
        config = {"recursion_limit": self._lg_config.recursion_limit}

        if self._lg_config.stream:
            return await self._stream_invoke(agent, native_input, config)

        # Prefer async
        if hasattr(agent, 'ainvoke'):
            return await agent.ainvoke(native_input, config=config)

        # Sync fallback (runs in threadpool to avoid blocking event loop)
        import asyncio
        return await asyncio.get_event_loop().run_in_executor(
            None, lambda: agent.invoke(native_input, config=config)
        )

    async def _stream_invoke(self, agent: Any, inp: dict, config: dict) -> Any:
        """Collect all streamed chunks and merge them."""
        result = {}
        if hasattr(agent, 'astream'):
            async for chunk in agent.astream(inp, config=config):
                if isinstance(chunk, dict):
                    result.update(chunk)
        return result or inp


# ── convenience function ──────────────────────────────────────────────────────

def wrap_langgraph(
    graph:   Any,
    name:    str,
    agent_id: str,
    owner:   str  = "0xYourWalletAddress",
    version: str  = "0.1.0",
    tags:    list[str] | None = None,
    tools:   list | None = None,
    expose_tools: bool = True,
    **kwargs,
) -> "WrappedAgent":
    """
    One-liner helper to wrap a LangGraph graph.

    Example:
      from plugins.langgraph_plugin import wrap_langgraph
      agent = wrap_langgraph(
          graph    = compiled_graph,
          name     = "MyResearcher",
          agent_id = "borgkit://agent/researcher",
          tags     = ["research", "web"],
      )
      await agent.register_discovery()
    """
    from plugins.base import WrappedAgent  # re-export type hint
    config = LangGraphPluginConfig(
        agent_id=agent_id, name=name, owner=owner,
        version=version, tags=tags or [],
        expose_tools_as_capabilities=expose_tools,
        **kwargs,
    )
    return LangGraphPlugin(config, tools=tools or []).wrap(graph)
