"""
AgnoPlugin — Borgkit adapter for Agno agents.

Wraps an Agno Agent so it is fully discoverable and callable on the
Borgkit mesh without any changes to the original agent code.

How it works
────────────
1. Capabilities are extracted from the agent's tools list at wrap time.
   Each tool function becomes one Borgkit capability.

2. AgentRequest payloads are translated into a plain string message for
   the Agno agent's .run() method.

3. The Agno run result (RunResponse) is mapped back to AgentResponse.

Install deps:
    pip install agno openai   # or your preferred model provider

Usage:
    from agno.agent import Agent
    from agno.models.openai import OpenAIChat
    from plugins.agno_plugin import wrap_agno

    def web_search(query: str) -> str:
        \"\"\"Search the web for information.\"\"\"
        return f"Search results for: {query}"

    agent = Agent(
        model=OpenAIChat(id="gpt-4o-mini"),
        tools=[web_search],
        description="Research assistant",
    )

    borgkit_agent = wrap_agno(
        agent    = agent,
        name     = "ResearchAgent",
        agent_id = "borgkit://agent/researcher",
        owner    = "0xYourWallet",
        tags     = ["research", "agno"],
    )
"""

from __future__ import annotations

import asyncio
import inspect
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


# ── Optional Agno import (soft dependency) ────────────────────────────────────

try:
    from agno.agent import Agent as AgnoAgent
    _AGNO_OK = True
except ImportError:
    _AGNO_OK = False
    AgnoAgent = Any  # type: ignore[assignment,misc]


# ── Plugin config ─────────────────────────────────────────────────────────────

@dataclass
class AgnoPluginConfig(PluginConfig):
    """Configuration for AgnoPlugin."""

    # Whether to stream the response (default: False — collect full output).
    stream: bool = False

    # Optional markdown flag passed to agent.run().
    markdown: bool = False

    # Maximum tokens the model may produce per call.
    max_tokens: Optional[int] = None


# ── Plugin ────────────────────────────────────────────────────────────────────

class AgnoPlugin(BorgkitPlugin):
    """
    Borgkit ↔ Agno bridge.

    Each tool registered on the Agno Agent becomes one Borgkit capability.
    Borgkit AgentRequests are translated into plain-text run inputs for the
    Agno Agent's .run() method.
    """

    def __init__(self, config: AgnoPluginConfig):
        if not _AGNO_OK:
            raise ImportError(
                "agno is not installed — run: pip install agno"
            )
        super().__init__(config)
        self._cfg: AgnoPluginConfig = config

    # ── BorgkitPlugin abstract methods ────────────────────────────────────────

    def extract_capabilities(self, agent: AgnoAgent) -> List[CapabilityDescriptor]:
        """Extract capabilities from the Agno agent's tool list."""
        caps: List[CapabilityDescriptor] = []

        tools = (
            getattr(agent, "tools", None)
            or getattr(agent, "_tools", None)
            or []
        )

        for t in tools:
            # Agno tools can be plain functions or callable objects
            fn = t if callable(t) else getattr(t, "entrypoint", None)
            if fn is None:
                continue

            name   = getattr(t, "name",  None) or getattr(fn, "__name__", str(t))
            desc   = getattr(t, "description", None) or (fn.__doc__ or "").strip()
            params = self._extract_params(fn)
            caps.append(CapabilityDescriptor(
                name        = name,
                description = desc,
                native_name = name,
                input_schema = {"type": "object", "properties": {
                    k: {"type": v} for k, v in params.items()
                }} if params else None,
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
        """Map an AgentRequest to an Agno run input dict."""
        message = (
            req.payload.get("message")
            or req.payload.get("query")
            or req.payload.get("task")
            or req.payload.get("input")
            or f"Perform the '{req.capability}' capability with: {req.payload}"
        )
        return {
            "message":    message,
            "capability": req.capability,
            "payload":    req.payload,
        }

    async def invoke_native(
        self,
        agent: AgnoAgent,
        descriptor: CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """Call agent.run() and return the RunResponse."""
        loop = asyncio.get_event_loop()
        result = await loop.run_in_executor(
            None,
            lambda: agent.run(
                native_input["message"],
                stream   = self._cfg.stream,
                markdown = self._cfg.markdown,
            ),
        )
        return result

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """Map a RunResponse (or string) to an AgentResponse."""
        # Agno RunResponse has a .content attribute
        if hasattr(native_result, "content"):
            content = native_result.content
        else:
            content = str(native_result)
        return AgentResponse.success(request_id, {"content": content})

    # ── helpers ───────────────────────────────────────────────────────────────

    @staticmethod
    def _extract_params(fn: Any) -> dict:
        params: dict = {}
        try:
            sig = inspect.signature(fn)
            for pname, param in sig.parameters.items():
                if pname in ("self", "cls"):
                    continue
                ann = param.annotation
                params[pname] = (
                    ann.__name__ if hasattr(ann, "__name__") else str(ann)
                ) if ann is not inspect.Parameter.empty else "Any"
        except (ValueError, TypeError):
            pass
        return params


# ── Convenience wrapper ───────────────────────────────────────────────────────

def wrap_agno(
    agent:      "AgnoAgent",
    name:       str,
    agent_id:   str,
    owner:      str,
    tags:       Optional[List[str]] = None,
    stream:     bool = False,
    markdown:   bool = False,
    max_tokens: Optional[int] = None,
    **kwargs:   Any,
) -> WrappedAgent:
    """
    Wrap an Agno Agent for the Borgkit mesh.

    After wrapping the agent:
      - exposes each tool as a Borgkit capability
      - registers with the configured discovery backend
      - handles AgentRequest / AgentResponse translation automatically

    Args:
        agent:      The Agno Agent instance to wrap.
        name:       Human-readable display name.
        agent_id:   Unique Borgkit URI, e.g. "borgkit://agent/researcher".
        owner:      Wallet or contract address.
        tags:       Optional search tags for discovery.
        stream:     Pass stream=True to agent.run() (default: False).
        markdown:   Pass markdown=True to agent.run() (default: False).
        max_tokens: Optional token limit per call.

    Returns:
        A WrappedAgent that implements IAgent.
    """
    cfg = AgnoPluginConfig(
        name       = name,
        agent_id   = agent_id,
        owner      = owner,
        tags       = tags or [],
        stream     = stream,
        markdown   = markdown,
        max_tokens = max_tokens,
    )
    return AgnoPlugin(cfg).wrap(agent)
