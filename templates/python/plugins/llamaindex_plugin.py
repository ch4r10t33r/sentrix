"""
LlamaIndexPlugin — Borgkit adapter for LlamaIndex agents.

Wraps a LlamaIndex agent (ReActAgent, OpenAIAgent, FunctionCallingAgent, etc.)
so it is fully discoverable and callable on the Borgkit mesh.

How it works
────────────
1. Capabilities are extracted from the agent's tool list (QueryPlanningTools,
   FunctionTools, etc.) at wrap time.

2. AgentRequest payloads are translated into a plain-text query string for
   the agent's .query() or .chat() method.

3. The LlamaIndex Response / AgentChatResponse is mapped back to AgentResponse.

Install deps:
    pip install llama-index llama-index-llms-openai

Usage:
    from llama_index.core.agent import ReActAgent
    from llama_index.core.tools import FunctionTool
    from llama_index.llms.openai import OpenAI
    from plugins.llamaindex_plugin import wrap_llamaindex

    def web_search(query: str) -> str:
        \"\"\"Search the web for information.\"\"\"
        return f"Search results for: {query}"

    tool  = FunctionTool.from_defaults(fn=web_search)
    llm   = OpenAI(model="gpt-4o-mini")
    agent = ReActAgent.from_tools([tool], llm=llm, verbose=False)

    borgkit_agent = wrap_llamaindex(
        agent    = agent,
        name     = "ResearchAgent",
        agent_id = "borgkit://agent/researcher",
        owner    = "0xYourWallet",
        tags     = ["research", "llamaindex"],
        tools    = [tool],
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


# ── Optional LlamaIndex import (soft dependency) ──────────────────────────────

try:
    from llama_index.core.agent.types import BaseAgent
    _LLAMAINDEX_OK = True
except ImportError:
    try:
        from llama_index.agent.types import BaseAgent  # older llama_index
        _LLAMAINDEX_OK = True
    except ImportError:
        _LLAMAINDEX_OK = False
        BaseAgent = Any  # type: ignore[assignment,misc]


# ── Plugin config ─────────────────────────────────────────────────────────────

@dataclass
class LlamaIndexPluginConfig(PluginConfig):
    """Configuration for LlamaIndexPlugin."""

    # Invoke via .chat() (conversational) or .query() (single-turn). Default: chat.
    invoke_method: str = "chat"

    # Optional tools list for explicit capability discovery (faster than introspection).
    tools: Optional[List[Any]] = None


# ── Plugin ────────────────────────────────────────────────────────────────────

class LlamaIndexPlugin(BorgkitPlugin):
    """
    Borgkit ↔ LlamaIndex bridge.

    Supports ReActAgent, OpenAIAgent, FunctionCallingAgent, and any other agent
    that subclasses BaseAgent and exposes .chat() or .query().
    """

    def __init__(self, config: LlamaIndexPluginConfig):
        if not _LLAMAINDEX_OK:
            raise ImportError(
                "llama-index is not installed — run: pip install llama-index"
            )
        super().__init__(config)
        self._cfg: LlamaIndexPluginConfig = config

    # ── BorgkitPlugin abstract methods ────────────────────────────────────────

    def extract_capabilities(self, agent: BaseAgent) -> List[CapabilityDescriptor]:
        """
        Extract capabilities from tools.

        Priority:
          1. Explicit tools list in config (fastest, most reliable)
          2. agent.tools / agent._tools attribute introspection
        """
        caps: List[CapabilityDescriptor] = []

        tools = (
            self._cfg.tools
            or getattr(agent, "tools", None)
            or getattr(agent, "_tools", None)
            or []
        )

        for t in tools:
            meta = getattr(t, "metadata", None)
            name = (
                getattr(meta, "name", None)
                if meta is not None
                else None
            ) or getattr(t, "name", None) or getattr(t, "__name__", str(t))

            desc = (
                getattr(meta, "description", None)
                if meta is not None
                else None
            ) or getattr(t, "description", "")

            # FunctionTool wraps a Python function; extract params from it
            fn     = getattr(t, "fn", None) or getattr(t, "_fn", None)
            params = self._extract_params(fn) if fn else {}

            input_schema = {"type": "object", "properties": {
                k: {"type": v} for k, v in params.items()
            }} if params else None

            caps.append(CapabilityDescriptor(
                name        = str(name),
                description = str(desc),
                native_name = str(name),
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
        """Map an AgentRequest to a query/message string."""
        message = (
            req.payload.get("query")
            or req.payload.get("message")
            or req.payload.get("input")
            or req.payload.get("task")
            or f"Perform '{req.capability}' with: {req.payload}"
        )
        return {
            "message":    message,
            "capability": req.capability,
            "payload":    req.payload,
        }

    async def invoke_native(
        self,
        agent: BaseAgent,
        descriptor: CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """
        Call .chat() or .query() on the LlamaIndex agent.

        .chat() is preferred for conversational agents (OpenAIAgent, ReActAgent).
        .query() is used for single-turn query engines.
        """
        method = self._cfg.invoke_method
        loop = asyncio.get_event_loop()

        if method == "query" and hasattr(agent, "query"):
            result = await loop.run_in_executor(
                None, lambda: agent.query(native_input["message"])
            )
        elif hasattr(agent, "chat"):
            result = await loop.run_in_executor(
                None, lambda: agent.chat(native_input["message"])
            )
        else:
            # Fallback: direct call
            result = await loop.run_in_executor(
                None, lambda: agent(native_input["message"])
            )
        return result

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """
        Map an AgentChatResponse or Response to AgentResponse.

        AgentChatResponse has .response (str).
        Response / QueryBundle has .response (str).
        """
        if hasattr(native_result, "response"):
            content = native_result.response
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

def wrap_llamaindex(
    agent:         "BaseAgent",
    name:          str,
    agent_id:      str,
    owner:         str,
    tags:          Optional[List[str]] = None,
    tools:         Optional[List[Any]] = None,
    invoke_method: str = "chat",
    **kwargs:      Any,
) -> WrappedAgent:
    """
    Wrap a LlamaIndex agent for the Borgkit mesh.

    Args:
        agent:         The LlamaIndex agent instance.
        name:          Human-readable display name.
        agent_id:      Unique Borgkit URI.
        owner:         Wallet or contract address.
        tags:          Optional search tags.
        tools:         Explicit tool list for capability discovery (recommended).
        invoke_method: "chat" (default) or "query".

    Returns:
        A WrappedAgent that implements IAgent.
    """
    cfg = LlamaIndexPluginConfig(
        name          = name,
        agent_id      = agent_id,
        owner         = owner,
        tags          = tags or [],
        tools         = tools,
        invoke_method = invoke_method,
    )
    return LlamaIndexPlugin(cfg).wrap(agent)
