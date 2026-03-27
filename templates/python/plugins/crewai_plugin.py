"""
CrewAIPlugin — Borgkit adapter for CrewAI agents.

Wraps a CrewAI Agent so it becomes fully discoverable and callable on the
Borgkit mesh without any changes to the original agent code.

How it works
────────────
1. At wrap time, capabilities are extracted from the agent's tool list.
   Each @tool-decorated function becomes one Borgkit capability.

2. When an AgentRequest arrives, translate_request() maps the Borgkit
   payload into a CrewAI Task description.

3. invoke_native() runs the task using a single-agent Crew, capturing
   the CrewOutput result.

4. translate_response() maps the CrewOutput back to an AgentResponse.

Install deps:
    pip install crewai crewai-tools

Usage:
    from plugins.crewai_plugin import wrap_crewai

    @tool("web_search")
    def web_search(query: str) -> str:
        \"\"\"Search the web for recent information.\"\"\"
        return f"Results for: {query}"

    agent = CrewAgent(
        role="Researcher",
        goal="Find accurate information",
        backstory="Expert research assistant.",
        tools=[web_search],
    )

    borgkit_agent = wrap_crewai(
        agent    = agent,
        name     = "ResearchAgent",
        agent_id = "borgkit://agent/researcher",
        owner    = "0xYourWallet",
        tags     = ["research", "crewai"],
    )
"""

from __future__ import annotations

from typing import Any, List, Optional

from plugins.base import (
    BorgkitPlugin,
    PluginConfig,
    CapabilityDescriptor,
    WrappedAgent,
)
from interfaces.agent_request  import AgentRequest
from interfaces.agent_response import AgentResponse


# ── Optional CrewAI import (soft dependency) ──────────────────────────────────

try:
    from crewai import Agent as CrewAgent, Task, Crew, Process
    from crewai.tools import BaseTool
    _CREWAI_OK = True
except ImportError:
    _CREWAI_OK = False
    CrewAgent = Any   # type: ignore[assignment,misc]


# ── Plugin config ─────────────────────────────────────────────────────────────

class CrewAIPluginConfig(PluginConfig):
    """
    Configuration for CrewAIPlugin.

    Inherits all fields from PluginConfig and adds CrewAI-specific options.
    """

    # CrewAI process type for running tasks.
    # Process.sequential runs tasks one at a time (default; safest for single-task use).
    process: str = "sequential"

    # Maximum number of iterations the crew is allowed per task.
    max_iter: int = 5

    # If True, the crew will emit verbose step-by-step output.
    verbose: bool = False

    # Optional system prompt prefix injected into every task description.
    system_prompt: Optional[str] = None


# ── Plugin ────────────────────────────────────────────────────────────────────

class CrewAIPlugin(BorgkitPlugin):
    """
    Borgkit ↔ CrewAI bridge.

    Each CrewAI tool decorated with @tool becomes one Borgkit capability.
    Borgkit AgentRequests are translated into CrewAI Tasks and run via a
    single-agent Crew.  Results are mapped back to AgentResponse.
    """

    def __init__(self, config: CrewAIPluginConfig):
        if not _CREWAI_OK:
            raise ImportError(
                "crewai is not installed — run: pip install crewai crewai-tools"
            )
        super().__init__(config)
        self._cfg: CrewAIPluginConfig = config

    # ── BorgkitPlugin abstract methods ────────────────────────────────────────

    def extract_capabilities(self, agent: CrewAgent) -> List[CapabilityDescriptor]:
        """
        Extract capabilities from the agent's tool list.

        Supports tools created with @tool decorator (crewai.tools.tool) and
        subclasses of BaseTool.
        """
        caps: List[CapabilityDescriptor] = []
        for t in (agent.tools or []):
            name = self._tool_name(t)
            desc = self._tool_description(t)
            params = self._tool_params(t)
            caps.append(CapabilityDescriptor(
                name        = name,
                description = desc,
                parameters  = params,
            ))
        return caps

    def translate_request(self, req: AgentRequest, agent: CrewAgent) -> dict:
        """
        Map a Borgkit AgentRequest to a CrewAI task description dict.

        The task description is built from:
        - A system prompt prefix (if configured)
        - The capability name
        - The full JSON payload
        """
        payload_str = _payload_to_string(req.payload)

        # Prefer an explicit 'task' or 'query' key in the payload
        task_body = (
            req.payload.get("task")
            or req.payload.get("query")
            or req.payload.get("input")
            or payload_str
        )

        prefix = self._cfg.system_prompt or f"You are performing the '{req.capability}' capability."
        description = f"{prefix}\n\nTask: {task_body}"

        return {
            "description":      description,
            "capability":       req.capability,
            "payload":          req.payload,
            "expected_output":  f"A complete result for the '{req.capability}' request.",
        }

    def invoke_native(self, agent: CrewAgent, translated: dict) -> Any:
        """
        Run the task using a single-agent Crew and return the raw CrewOutput.

        Each AgentRequest gets its own Crew instance.  CrewAI's shared state
        is reset between calls so there is no cross-request contamination.
        """
        task = Task(
            description     = translated["description"],
            agent           = agent,
            expected_output = translated["expected_output"],
        )

        crew = Crew(
            agents  = [agent],
            tasks   = [task],
            process = Process.sequential,
            verbose = self._cfg.verbose,
        )

        result = crew.kickoff()
        return result

    def translate_response(self, native_output: Any, req: AgentRequest) -> AgentResponse:
        """
        Map a CrewOutput (or plain string) back to a Borgkit AgentResponse.
        """
        # CrewOutput has .raw (str) and .tasks_output (list) attributes
        if hasattr(native_output, "raw"):
            content = native_output.raw
        else:
            content = str(native_output)

        return AgentResponse.success(req.request_id, {"content": content})

    # ── Tool introspection helpers ────────────────────────────────────────────

    @staticmethod
    def _tool_name(tool: Any) -> str:
        """Extract a clean capability name from a crewai tool."""
        # @tool-decorated functions expose .name
        if hasattr(tool, "name"):
            return str(tool.name)
        # BaseTool subclasses expose .name as a class attribute
        if hasattr(tool, "__class__") and hasattr(tool.__class__, "name"):
            return str(tool.__class__.name)
        return getattr(tool, "__name__", str(tool))

    @staticmethod
    def _tool_description(tool: Any) -> str:
        """Extract a human-readable description from a crewai tool."""
        if hasattr(tool, "description"):
            return str(tool.description)
        if hasattr(tool, "__doc__") and tool.__doc__:
            return tool.__doc__.strip()
        return ""

    @staticmethod
    def _tool_params(tool: Any) -> dict:
        """
        Best-effort extraction of parameter schema from a crewai tool.
        Returns a dict of {param_name: type_hint_string}.
        """
        import inspect
        params: dict = {}
        fn = getattr(tool, "_run", None) or getattr(tool, "func", None) or tool
        if callable(fn):
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

def wrap_crewai(
    agent:    "CrewAgent",
    name:     str,
    agent_id: str,
    owner:    str,
    tags:     Optional[List[str]] = None,
    process:  str = "sequential",
    verbose:  bool = False,
    system_prompt: Optional[str] = None,
    **kwargs: Any,
) -> WrappedAgent:
    """
    Wrap a CrewAI Agent for the Borgkit mesh.

    After wrapping, the agent:
      - exposes each @tool-decorated function as a Borgkit capability
      - registers with LocalDiscovery (or whichever backend is configured)
      - handles AgentRequest / AgentResponse translation automatically
      - can be discovered by any other Borgkit agent via capability query

    Args:
        agent:         The CrewAI Agent instance to wrap.
        name:          Human-readable name shown in discovery results.
        agent_id:      Unique Borgkit URI, e.g. "borgkit://agent/researcher".
        owner:         Wallet or contract address of the agent owner.
        tags:          Optional list of search tags for discovery.
        process:       CrewAI Process type ("sequential" or "hierarchical").
        verbose:       Enable CrewAI verbose output.
        system_prompt: Optional prefix injected into every task description.

    Returns:
        A WrappedAgent that implements IAgent and is ready for registration.

    Example:
        from crewai import Agent as CrewAgent
        from crewai.tools import tool

        @tool("summarise")
        def summarise(text: str) -> str:
            \"\"\"Summarise a block of text into key points.\"\"\"
            return f"Summary: {text[:100]}..."

        agent = CrewAgent(
            role      = "Summariser",
            goal      = "Produce concise summaries",
            backstory = "Expert at distilling information.",
            tools     = [summarise],
        )

        wrapped = wrap_crewai(
            agent    = agent,
            name     = "SummaryAgent",
            agent_id = "borgkit://agent/summariser",
            owner    = "0xYourWallet",
            tags     = ["summarise", "nlp", "crewai"],
        )
    """
    cfg = CrewAIPluginConfig(
        name          = name,
        agent_id      = agent_id,
        owner         = owner,
        tags          = tags or [],
        process       = process,
        verbose       = verbose,
        system_prompt = system_prompt,
        **{k: v for k, v in kwargs.items()
           if k in ("version", "metadata_uri", "heartbeat_interval_s")},
    )
    plugin = CrewAIPlugin(cfg)
    return plugin.wrap(agent)


# ── Helpers ───────────────────────────────────────────────────────────────────

def _payload_to_string(payload: dict) -> str:
    """Convert an AgentRequest payload dict to a compact string for task descriptions."""
    import json
    try:
        return json.dumps(payload, ensure_ascii=False, indent=None)
    except (TypeError, ValueError):
        return str(payload)
