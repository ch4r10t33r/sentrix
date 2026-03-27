"""
SmolagentsPlugin — Borgkit adapter for smolagents agents.

Wraps a smolagents agent (CodeAgent or ToolCallingAgent) so it is fully
discoverable and callable on the Borgkit mesh.

How it works
────────────
1. Capabilities are extracted from the agent's toolbox at wrap time.
   Each @tool-decorated function becomes one Borgkit capability.

2. AgentRequest payloads are translated into a plain-text task string.

3. The agent's .run() output (string) is mapped back to AgentResponse.

Install deps:
    pip install smolagents

Usage:
    from smolagents import ToolCallingAgent, tool
    from smolagents.models import HfApiModel
    from plugins.smolagents_plugin import wrap_smolagents

    @tool
    def web_search(query: str) -> str:
        \"\"\"Search the web for information.
        Args:
            query: The search query.
        Returns:
            Search results as a string.
        \"\"\"
        return f"Search results for: {query}"

    agent = ToolCallingAgent(
        tools=[web_search],
        model=HfApiModel("Qwen/Qwen2.5-72B-Instruct"),
    )

    borgkit_agent = wrap_smolagents(
        agent    = agent,
        name     = "ResearchAgent",
        agent_id = "borgkit://agent/researcher",
        owner    = "0xYourWallet",
        tags     = ["research", "smolagents"],
    )
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from typing import Any, List, Optional

from plugins.base import (
    BorgkitPlugin,
    PluginConfig,
    CapabilityDescriptor,
    WrappedAgent,
)
from interfaces.agent_request  import AgentRequest
from interfaces.agent_response import AgentResponse


# ── Optional smolagents import (soft dependency) ──────────────────────────────

try:
    from smolagents.agents import BaseAgent as SmolagentsBaseAgent
    _SMOLAGENTS_OK = True
except ImportError:
    _SMOLAGENTS_OK = False
    SmolagentsBaseAgent = Any  # type: ignore[assignment,misc]


# ── Plugin config ─────────────────────────────────────────────────────────────

@dataclass
class SmolagentsPluginConfig(PluginConfig):
    """Configuration for SmolagentsPlugin."""

    # Additional kwargs passed to agent.run().
    run_kwargs: Optional[dict] = None


# ── Plugin ────────────────────────────────────────────────────────────────────

class SmolagentsPlugin(BorgkitPlugin):
    """
    Borgkit ↔ smolagents bridge.

    Supports both CodeAgent and ToolCallingAgent.  Each @tool-decorated
    function in the agent's toolbox becomes one Borgkit capability.
    """

    def __init__(self, config: SmolagentsPluginConfig):
        if not _SMOLAGENTS_OK:
            raise ImportError(
                "smolagents is not installed — run: pip install smolagents"
            )
        super().__init__(config)
        self._cfg: SmolagentsPluginConfig = config

    # ── BorgkitPlugin abstract methods ────────────────────────────────────────

    def extract_capabilities(self, agent: SmolagentsBaseAgent) -> List[CapabilityDescriptor]:
        """
        Extract capabilities from the agent's toolbox.

        smolagents stores tools in agent.tools (dict: name → Tool) or
        agent.toolbox (ToolCollection).
        """
        caps: List[CapabilityDescriptor] = []

        # agent.tools is a dict {name: Tool} in recent smolagents versions
        tools_dict: dict = {}
        if hasattr(agent, "tools") and isinstance(agent.tools, dict):
            tools_dict = agent.tools
        elif hasattr(agent, "toolbox"):
            tb = agent.toolbox
            tools_dict = getattr(tb, "tools", {}) or {}

        for tool_name, tool_obj in tools_dict.items():
            desc   = getattr(tool_obj, "description", "") or ""
            inputs = getattr(tool_obj, "inputs", {}) or {}
            params = {
                k: (v.get("type", "Any") if isinstance(v, dict) else str(v))
                for k, v in inputs.items()
            }
            input_schema = {"type": "object", "properties": {
                k: {"type": v} for k, v in params.items()
            }} if params else None
            caps.append(CapabilityDescriptor(
                name        = tool_name,
                description = desc,
                native_name = tool_name,
                input_schema = input_schema,
            ))

        if not caps:
            # Single-invocation fallback
            caps.append(CapabilityDescriptor(
                name        = "invoke",
                description = getattr(agent, "description", "") or self._cfg.description,
                native_name = "__agent__",
                tags        = list(self._cfg.tags),
            ))

        return caps

    def translate_request(
        self,
        req: AgentRequest,
        descriptor: CapabilityDescriptor,
    ) -> dict:
        """Map an AgentRequest to a task string for agent.run()."""
        task = (
            req.payload.get("task")
            or req.payload.get("query")
            or req.payload.get("message")
            or req.payload.get("input")
            or f"Perform the '{req.capability}' capability. Payload: {req.payload}"
        )
        return {
            "task":       task,
            "capability": req.capability,
            "payload":    req.payload,
        }

    async def invoke_native(
        self,
        agent: SmolagentsBaseAgent,
        descriptor: CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """Call agent.run(task) and return the result string."""
        run_kwargs = self._cfg.run_kwargs or {}
        loop = asyncio.get_event_loop()
        result = await loop.run_in_executor(
            None,
            lambda: agent.run(native_input["task"], **run_kwargs),
        )
        return result

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """Map the agent.run() return value (typically a string) to AgentResponse."""
        content = str(native_result)
        return AgentResponse.success(request_id, {"content": content})


# ── Convenience wrapper ───────────────────────────────────────────────────────

def wrap_smolagents(
    agent:      "SmolagentsBaseAgent",
    name:       str,
    agent_id:   str,
    owner:      str,
    tags:       Optional[List[str]] = None,
    run_kwargs: Optional[dict] = None,
    **kwargs:   Any,
) -> WrappedAgent:
    """
    Wrap a smolagents agent (CodeAgent or ToolCallingAgent) for the Borgkit mesh.

    Args:
        agent:      The smolagents agent instance.
        name:       Human-readable display name.
        agent_id:   Unique Borgkit URI, e.g. "borgkit://agent/researcher".
        owner:      Wallet or contract address.
        tags:       Optional search tags for discovery.
        run_kwargs: Extra keyword arguments forwarded to agent.run().

    Returns:
        A WrappedAgent that implements IAgent.

    Example::

        from smolagents import ToolCallingAgent, tool
        from smolagents.models import HfApiModel

        @tool
        def summarise(text: str) -> str:
            \"\"\"Summarise text.
            Args:
                text: Text to summarise.
            Returns:
                Summary.
            \"\"\"
            return text[:200] + "..."

        agent = ToolCallingAgent(tools=[summarise], model=HfApiModel("..."))

        wrapped = wrap_smolagents(
            agent    = agent,
            name     = "SummaryAgent",
            agent_id = "borgkit://agent/summariser",
            owner    = "0xYourWallet",
            tags     = ["summarise", "smolagents"],
        )
    """
    cfg = SmolagentsPluginConfig(
        name       = name,
        agent_id   = agent_id,
        owner      = owner,
        tags       = tags or [],
        run_kwargs = run_kwargs,
    )
    return SmolagentsPlugin(cfg).wrap(agent)
