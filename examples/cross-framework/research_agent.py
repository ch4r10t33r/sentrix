"""
ResearchAgent — Google ADK, wrapped for the Borgkit mesh.
──────────────────────────────────────────────────────────
Capability: research_topic(topic, depth) → structured findings

In DEMO mode (no GOOGLE_API_KEY):  tool functions run directly.
In LIVE mode (GOOGLE_API_KEY set): Google ADK routes via Gemini 2.0 Flash.
"""
from __future__ import annotations
import os, sys, json
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../../templates/python"))

DEMO_MODE = not bool(os.getenv("GOOGLE_API_KEY"))

# ── Tool functions ────────────────────────────────────────────────────────────
# These contain the real capability logic.
# In demo mode they return realistic mock data.
# In live mode the ADK agent uses Gemini to decide when/how to call them.

def research_topic(topic: str, depth: str = "standard") -> str:
    """
    Research a topic and return structured findings.

    Args:
        topic: The subject to research.
        depth: "brief" | "standard" | "deep"
    """
    if DEMO_MODE:
        return json.dumps({
            "topic":    topic,
            "depth":    depth,
            "summary":  (
                f"{topic} is a rapidly evolving field with significant implications "
                "for industry and society. Recent research highlights three key areas "
                "of development: scalability, interpretability, and cross-system interoperability."
            ),
            "key_findings": [
                f"Adoption of {topic} has grown 340% year-over-year in enterprise settings",
                "Leading approaches share a common focus on modularity and composability",
                "Regulatory frameworks are still catching up with technical capabilities",
                "Open-source tooling has accelerated experimentation across all sectors",
            ],
            "sources": [
                "Nature Machine Intelligence (2024)",
                "MIT Technology Review (2024)",
                "Gartner Hype Cycle for AI (2024)",
            ],
            "confidence": 0.87,
        })

    # ── Live implementation ───────────────────────────────────────────────────
    # Replace with real search + synthesis logic (e.g. Tavily, Exa, Perplexity)
    import httpx
    raise NotImplementedError(
        "Live research_topic not implemented. "
        "Wire up a search API (Tavily, Exa, etc.) here."
    )


def find_recent_papers(topic: str, max_results: int = 5) -> str:
    """
    Find recent academic papers on a topic.

    Args:
        topic:       Subject area to search.
        max_results: Maximum number of papers to return.
    """
    if DEMO_MODE:
        return json.dumps({
            "topic":  topic,
            "papers": [
                {
                    "title":   f"Advances in {topic}: A Systematic Review",
                    "authors": ["A. Smith", "B. Jones"],
                    "year":    2024,
                    "url":     "https://arxiv.org/abs/2401.00001",
                    "abstract": f"We survey recent advances in {topic}, covering 142 papers from 2022–2024.",
                },
                {
                    "title":   f"Benchmarking {topic} Systems at Scale",
                    "authors": ["C. Lee", "D. Kumar"],
                    "year":    2024,
                    "url":     "https://arxiv.org/abs/2401.00002",
                    "abstract": f"We introduce a new benchmark suite for evaluating {topic} systems.",
                },
            ][:max_results],
        })
    raise NotImplementedError("Live find_recent_papers not implemented.")


# ── Build the Google ADK agent ────────────────────────────────────────────────

try:
    from google.adk.agents import Agent
    from google.adk.tools  import FunctionTool
    _ADK_AVAILABLE = True
except ImportError:
    _ADK_AVAILABLE = False


if _ADK_AVAILABLE:
    _adk_agent = Agent(
        name        = "research_agent",
        model       = "gemini-2.0-flash",
        description = "Research agent: finds facts, trends, and papers on any topic.",
        tools       = [FunctionTool(research_topic), FunctionTool(find_recent_papers)],
    )
else:
    _adk_agent = None


# ── Demo-mode plugin: calls tool functions directly without ADK Runner ────────

class _DemoAwareGoogleADKPlugin:
    """
    Thin wrapper used in demo mode.
    Exposes the same interface as GoogleADKPlugin but calls tool functions
    directly — no Gemini API key required.
    In live mode, replaced by the real GoogleADKPlugin.
    """

    def __init__(self, tools: dict):
        self._tools = tools  # {name: callable}

    def invoke(self, capability: str, payload: dict) -> str:
        fn = self._tools.get(capability)
        if fn is None:
            return json.dumps({"error": f"Unknown capability: {capability}"})
        try:
            return fn(**{k: v for k, v in payload.items() if k in fn.__code__.co_varnames})
        except Exception as e:
            return json.dumps({"error": str(e)})


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────

def build_research_agent():
    """
    Build and return a Borgkit-wrapped ResearchAgent.

    Returns a WrappedAgent (implements IAgent) ready for:
        await agent.register_discovery()
        await agent.handle_request(req)
    """
    if DEMO_MODE or not _ADK_AVAILABLE:
        # Demo path: use a simple IAgent that calls tool functions directly.
        # Shows full Borgkit plumbing (capability extraction, discovery, A2A calling)
        # without needing a GOOGLE_API_KEY.
        from interfaces.iagent          import IAgent
        from interfaces.agent_request   import AgentRequest
        from interfaces.agent_response  import AgentResponse
        from discovery.local_discovery  import LocalDiscovery
        from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
        from datetime import datetime, timezone

        _TOOLS = {
            "research_topic":    research_topic,
            "find_recent_papers": find_recent_papers,
        }

        class ResearchAgentDemo(IAgent):
            agent_id    = "borgkit://agent/research"
            owner       = "0xResearchOwner"

            def get_capabilities(self):
                return list(_TOOLS.keys())

            async def handle_request(self, req: AgentRequest) -> AgentResponse:
                fn = _TOOLS.get(req.capability)
                if fn is None:
                    return AgentResponse.error(req.request_id, f"Unknown capability: {req.capability}")
                try:
                    raw = fn(**req.payload)
                    result = json.loads(raw) if isinstance(raw, str) else raw
                    return AgentResponse.success(req.request_id, {"content": result})
                except Exception as e:
                    return AgentResponse.error(req.request_id, str(e))

            def get_anr(self):
                now = datetime.now(timezone.utc).isoformat()
                return DiscoveryEntry(
                    agent_id      = self.agent_id,
                    name          = "ResearchAgent",
                    owner         = self.owner,
                    capabilities  = self.get_capabilities(),
                    network       = NetworkInfo(protocol="http", host="localhost", port=8081),
                    health        = HealthStatus(status="healthy", last_heartbeat=now),
                    registered_at = now,
                    metadata_uri  = None,
                )

            async def register_discovery(self, discovery=None) -> None:
                from discovery.local_discovery import LocalDiscovery
                reg = discovery or LocalDiscovery.get_instance()
                await reg.register(self.get_anr())

        return ResearchAgentDemo()

    else:
        # Live path: real GoogleADKPlugin → Gemini routes tool calls
        from plugins.google_adk_plugin import wrap_google_adk
        return wrap_google_adk(
            agent    = _adk_agent,
            name     = "ResearchAgent",
            agent_id = "borgkit://agent/research",
            owner    = "0xResearchOwner",
            tags     = ["research", "adk", "gemini"],
            port     = 8081,
        )
