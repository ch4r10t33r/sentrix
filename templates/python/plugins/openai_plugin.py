"""
OpenAI Agents SDK → Sentrix Plugin
──────────────────────────────────────────────────────────────────────────────
Wraps any OpenAI Agents SDK `Agent` so it appears as a standard Sentrix IAgent
on the mesh — discoverable, callable by other agents, and serveable over HTTP.

Capability extraction strategy (priority order)
────────────────────────────────────────────────
  1. Explicit `capability_map` in PluginConfig
  2. Agent's `tools` list  — each @function_tool becomes one capability
  3. Agent's `handoffs`    — each referenced Agent becomes one capability
  4. Single-invocation fallback: one `invoke` capability = the whole agent

Invocation
──────────
  Tool-specific  → Runner.run(agent, "Use tool X with args: {…}")
  Handoff target → Runner.run(target_agent, message)
  Full agent     → Runner.run(agent, message)

Requirements
────────────
  pip install openai-agents
  export OPENAI_API_KEY=sk-...

Usage
─────
  from plugins.openai_plugin import OpenAIPlugin, OpenAIPluginConfig
  from agents import Agent, function_tool

  @function_tool
  def web_search(query: str) -> str:
      \"\"\"Search the web for up-to-date information.\"\"\"
      return f"Results for: {query}"

  oai_agent = Agent(
      name="WebSearchAgent",
      instructions="You are a helpful web researcher.",
      tools=[web_search],
      model="gpt-4o-mini",
  )

  config = OpenAIPluginConfig(
      agent_id="sentrix://agent/web-search",
      name="WebSearchAgent",
      owner="0xYourWallet",
      port=8082,
  )
  plugin = OpenAIPlugin(config)
  agent  = plugin.wrap(oai_agent)

  # One-liner shortcut:
  # from plugins.openai_plugin import wrap_openai
  # agent = wrap_openai(oai_agent, name="WebSearchAgent", agent_id="sentrix://agent/web-search", owner="0xWallet")

  await agent.serve(port=8082)
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Optional

from interfaces import AgentRequest, AgentResponse
from plugins.base import SentrixPlugin, CapabilityDescriptor, PluginConfig, WrappedAgent


# ── extended config ────────────────────────────────────────────────────────────

@dataclass
class OpenAIPluginConfig(PluginConfig):
    """OpenAI Agents SDK-specific configuration (extends PluginConfig)."""

    # Expose each @function_tool as a separate Sentrix capability.
    # When False, a single "invoke" capability wraps the whole agent.
    expose_tools_as_capabilities: bool = True

    # Expose handoff targets (sub-agents) as additional capabilities.
    expose_handoffs_as_capabilities: bool = True

    # Maximum agentic turns per request (guards against runaway loops).
    max_turns: int = 10

    # Optional model override. When set, overrides the Agent's own model.
    # Useful for testing: swap in a cheaper/faster model per deployment.
    model_override: Optional[str] = None

    # Tracing / context name shown in OpenAI traces dashboard.
    trace_name: str = "sentrix"


# ── native I/O types ───────────────────────────────────────────────────────────

@dataclass
class _OpenAINativeInput:
    message:          str
    target_agent:     Any = None          # Agent object for handoff dispatch
    target_agent_name: str = ""


# ── plugin ─────────────────────────────────────────────────────────────────────

class OpenAIPlugin(SentrixPlugin):
    """
    Sentrix plugin for the OpenAI Agents SDK.

    Wraps any ``agents.Agent`` and bridges it into the Sentrix mesh.
    Each tool and handoff target becomes a discoverable capability.
    """

    # Prefix used to mark native_name for handoff capabilities
    _HANDOFF_PREFIX = "__handoff__:"

    def __init__(self, config: OpenAIPluginConfig) -> None:
        super().__init__(config)
        self._cfg: OpenAIPluginConfig = config

    # ── capability extraction ──────────────────────────────────────────────────

    def extract_capabilities(self, agent: Any) -> list[CapabilityDescriptor]:
        caps: list[CapabilityDescriptor] = []

        # 1. Explicit capability_map overrides everything
        if self.config.capability_map:
            for sentrix_name, native_name in self.config.capability_map.items():
                caps.append(CapabilityDescriptor(
                    name=sentrix_name,
                    description=f"Mapped capability → {native_name}",
                    native_name=native_name,
                ))
            return caps

        # 2. Tools
        if self._cfg.expose_tools_as_capabilities:
            for tool in _get_tools(agent):
                caps.append(_tool_to_descriptor(tool))

        # 3. Handoff targets (sub-agents)
        if self._cfg.expose_handoffs_as_capabilities:
            for handoff in _get_handoffs(agent):
                target = _handoff_agent(handoff)
                if target is None:
                    continue
                name = _sanitize(getattr(target, "name", "") or "handoff")
                caps.append(CapabilityDescriptor(
                    name=name,
                    description=(
                        getattr(target, "instructions", "")[:120]
                        or f"Handoff to sub-agent '{name}'"
                    ),
                    native_name=f"{self._HANDOFF_PREFIX}{name}",
                    tags=["handoff", "sub-agent"],
                ))

        # 4. Fallback: one capability = whole agent
        if not caps:
            caps.append(CapabilityDescriptor(
                name="invoke",
                description=(
                    getattr(agent, "instructions", "")[:120]
                    or self._cfg.description
                    or f"Invoke the {self._cfg.name} agent"
                ),
                native_name="__agent__",
                tags=self._cfg.tags,
            ))

        return caps

    # ── request translation ────────────────────────────────────────────────────

    def translate_request(
        self,
        req:        AgentRequest,
        descriptor: CapabilityDescriptor,
    ) -> _OpenAINativeInput:
        native = (
            self.config.capability_map.get(req.capability)
            or descriptor.native_name
            or req.capability
        )

        # ── whole-agent or generic invocation ──────────────────────────────────
        if native == "__agent__":
            message = (
                req.payload.get("message")
                or req.payload.get("input")
                or req.payload.get("query")
                or json.dumps(req.payload, ensure_ascii=False)
            )
            return _OpenAINativeInput(message=message)

        # ── handoff target ─────────────────────────────────────────────────────
        if native.startswith(self._HANDOFF_PREFIX):
            target_name = native[len(self._HANDOFF_PREFIX):]
            message = (
                req.payload.get("message")
                or req.payload.get("input")
                or json.dumps(req.payload, ensure_ascii=False)
            )
            return _OpenAINativeInput(
                message=target_name,          # resolved to real agent in invoke_native
                target_agent_name=target_name,
            )

        # ── tool-specific ──────────────────────────────────────────────────────
        args_str = json.dumps(req.payload, indent=2, ensure_ascii=False)
        message = f"Use the `{native}` function with these arguments:\n{args_str}"
        return _OpenAINativeInput(message=message)

    # ── response translation ───────────────────────────────────────────────────

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        try:
            content = _extract_output(native_result)
            return AgentResponse.success(request_id, {
                "content": content,
                "raw":     _safe_serialize(native_result),
            })
        except Exception as exc:
            return AgentResponse.error(request_id, f"Response translation failed: {exc}")

    # ── native invocation ──────────────────────────────────────────────────────

    async def invoke_native(
        self,
        agent:        Any,
        descriptor:   CapabilityDescriptor,
        native_input: _OpenAINativeInput,
    ) -> Any:
        """
        Run the OpenAI agent via ``Runner.run()``.

        Dispatches to the correct target:
          - Handoff capability → run the named sub-agent
          - Tool / full-agent  → run the root agent with a crafted prompt
        """
        try:
            from agents import Runner
        except ImportError:
            raise RuntimeError(
                "openai-agents is not installed.\n"
                "Install: pip install openai-agents\n"
                "Also set: export OPENAI_API_KEY=sk-..."
            )

        # Resolve handoff target
        run_agent = agent
        if native_input.target_agent_name:
            for handoff in _get_handoffs(agent):
                target = _handoff_agent(handoff)
                if target and _sanitize(getattr(target, "name", "")) == native_input.target_agent_name:
                    run_agent = target
                    break
            else:
                raise ValueError(
                    f"Handoff target '{native_input.target_agent_name}' not found on agent "
                    f"'{getattr(agent, 'name', '?')}'. "
                    f"Available: {[_sanitize(getattr(_handoff_agent(h), 'name', '')) for h in _get_handoffs(agent)]}"
                )

        # Apply model override if requested
        if self._cfg.model_override:
            try:
                run_agent = run_agent.clone(model=self._cfg.model_override)
            except Exception:
                pass  # clone not available in all SDK versions; proceed as-is

        result = await Runner.run(
            run_agent,
            native_input.message,
            max_turns=self._cfg.max_turns,
        )
        return result


# ── helpers ────────────────────────────────────────────────────────────────────

def _get_tools(agent: Any) -> list:
    """Extract tools from an OpenAI Agents SDK Agent."""
    # SDK Agent stores tools in .tools (list of FunctionTool / BaseTool)
    return list(getattr(agent, "tools", None) or [])


def _get_handoffs(agent: Any) -> list:
    """Extract handoff references from an Agent."""
    return list(getattr(agent, "handoffs", None) or [])


def _handoff_agent(handoff: Any) -> Any:
    """
    Unwrap a handoff reference to the target Agent.

    The SDK allows handoffs to be specified as:
      - An Agent instance directly
      - A Handoff object with an .agent attribute
      - A callable returning an Agent
    """
    # Direct Agent
    if hasattr(handoff, "name") and hasattr(handoff, "tools"):
        return handoff
    # Handoff object
    target = getattr(handoff, "agent", None)
    if target is not None:
        return target
    # Callable (lazy handoff)
    if callable(handoff):
        try:
            return handoff()
        except Exception:
            pass
    return None


def _tool_to_descriptor(tool: Any) -> CapabilityDescriptor:
    """Convert an OpenAI Agents SDK tool to a CapabilityDescriptor."""
    # @function_tool / FunctionTool exposes .name and .description
    name = getattr(tool, "name", None) or getattr(tool, "__name__", str(tool))
    desc = getattr(tool, "description", "") or ""

    # FunctionTool stores the JSON schema in .params_json_schema
    input_schema: Optional[dict] = None
    try:
        schema = getattr(tool, "params_json_schema", None)
        if isinstance(schema, dict):
            input_schema = schema
    except Exception:
        pass

    # Fallback: inspect the wrapped function's annotations
    if not input_schema:
        try:
            fn = getattr(tool, "fn", None) or getattr(tool, "_fn", None)
            if fn and hasattr(fn, "__annotations__"):
                props = {
                    k: {"type": "string"}
                    for k in fn.__annotations__
                    if k != "return"
                }
                if props:
                    input_schema = {"type": "object", "properties": props}
        except Exception:
            pass

    return CapabilityDescriptor(
        name=_sanitize(name),
        description=desc,
        native_name=name,
        input_schema=input_schema,
    )


def _extract_output(result: Any) -> str:
    """Extract the final string output from a RunResult."""
    # OpenAI Agents SDK RunResult has .final_output
    final = getattr(result, "final_output", None)
    if final is not None:
        return str(final)
    # Fallback attributes
    for attr in ("output", "response", "content", "text"):
        val = getattr(result, attr, None)
        if val is not None:
            return str(val)
    return str(result)


def _safe_serialize(obj: Any) -> Any:
    try:
        return json.loads(json.dumps(obj, default=str))
    except Exception:
        return str(obj)


def _sanitize(name: str) -> str:
    """Normalize to a valid Sentrix capability name."""
    return name.replace(" ", "_").replace("-", "_").lower()


# ── one-liner convenience wrapper ─────────────────────────────────────────────

def wrap_openai(
    agent:    Any,
    name:     str,
    agent_id: str,
    owner:    str            = "0xYourWalletAddress",
    version:  str            = "0.1.0",
    port:     int            = 6174,
    tags:     list[str] | None = None,
    expose_tools:    bool    = True,
    expose_handoffs: bool    = True,
    max_turns:       int     = 10,
    **kwargs,
) -> WrappedAgent:
    """
    Wrap an OpenAI Agents SDK Agent for the Sentrix mesh in one line.

    Example::

        from agents import Agent, function_tool
        from plugins.openai_plugin import wrap_openai

        @function_tool
        def get_weather(city: str) -> str:
            \"\"\"Return current weather for a city.\"\"\"
            return f"Sunny, 22°C in {city}"

        oai_agent = Agent(
            name="WeatherBot",
            instructions="Answer weather questions concisely.",
            tools=[get_weather],
        )

        sentrix_agent = wrap_openai(
            agent    = oai_agent,
            name     = "WeatherBot",
            agent_id = "sentrix://agent/weather",
            owner    = "0xYourWallet",
            tags     = ["weather", "openai"],
        )
        await sentrix_agent.serve(port=8082)

    Args:
        agent:           The ``agents.Agent`` instance to wrap.
        name:            Human-readable name shown in discovery.
        agent_id:        Unique Sentrix URI, e.g. ``"sentrix://agent/my-bot"``.
        owner:           Wallet address or identifier of the agent owner.
        version:         Semantic version string.
        port:            HTTP port for ``agent.serve()``.
        tags:            Search tags for capability discovery.
        expose_tools:    Expose each tool as a separate capability.
        expose_handoffs: Expose handoff targets as capabilities.
        max_turns:       Max agentic turns per request.

    Returns:
        A ``WrappedAgent`` implementing ``IAgent``, ready for ``serve()`` or
        ``register_discovery()``.
    """
    config = OpenAIPluginConfig(
        agent_id=agent_id,
        name=name,
        owner=owner,
        version=version,
        port=port,
        tags=tags or [],
        expose_tools_as_capabilities=expose_tools,
        expose_handoffs_as_capabilities=expose_handoffs,
        max_turns=max_turns,
        **{k: v for k, v in kwargs.items()
           if k in (
               "description", "metadata_uri", "discovery_type",
               "discovery_url", "signing_key", "x402_pricing",
               "model_override", "trace_name",
           )},
    )
    return OpenAIPlugin(config).wrap(agent)
