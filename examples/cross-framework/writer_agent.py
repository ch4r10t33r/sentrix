"""
WriterAgent — CrewAI, wrapped for the Borgkit mesh.
──────────────────────────────────────────────────────────
Capabilities:
  write_article(topic, research, style)  → polished article
  write_summary(content, max_words)      → condensed summary

In DEMO mode (no OPENAI_API_KEY):  tool functions run directly.
In LIVE mode (OPENAI_API_KEY set): CrewAI runs a full crew with GPT-4o-mini.
"""
from __future__ import annotations
import os, sys, json
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../../templates/python"))

DEMO_MODE = not bool(os.getenv("OPENAI_API_KEY"))

# ── Tool functions ────────────────────────────────────────────────────────────

def write_article(topic: str, research: str = "", style: str = "informative") -> str:
    """
    Write a polished article from research findings.

    Args:
        topic:    The article topic / headline.
        research: JSON string of research findings (from ResearchAgent).
        style:    "informative" | "persuasive" | "technical"
    """
    # Parse research if it's a JSON blob
    findings = {}
    if research:
        try:
            data = json.loads(research) if isinstance(research, str) else research
            # Handle nested {"content": {...}} from AgentResponse.result
            findings = data.get("content", data) if isinstance(data, dict) else {"raw": str(data)}
        except (json.JSONDecodeError, AttributeError):
            findings = {"raw": str(research)}

    summary   = findings.get("summary", "")
    key_points = findings.get("key_findings", [])
    sources   = findings.get("sources", [])

    if DEMO_MODE:
        points_text = "\n".join(f"  - {p}" for p in key_points) if key_points else "  - (no key findings provided)"
        sources_text = ", ".join(sources) if sources else "various sources"

        overview = summary or f"This article explores {topic} and its implications across industry and research."
        article = (
            f"# {topic}\n\n"
            f"## Overview\n\n{overview}\n\n"
            f"## Key Developments\n\n{points_text}\n\n"
            f"## Analysis\n\n"
            f"The convergence of these trends points to a pivotal moment for {topic}. "
            f"Practitioners who invest early in understanding the landscape will be "
            f"well-positioned as the field matures over the next 12–24 months.\n\n"
            f"## Conclusion\n\n"
            f"{topic} continues to evolve rapidly. Staying current requires engaging "
            f"with primary research, experimentation, and cross-disciplinary collaboration.\n\n"
            f"---\n*Sources: {sources_text}*"
        )

        return json.dumps({"article": article, "word_count": len(article.split()), "style": style})

    # ── Live implementation ───────────────────────────────────────────────────
    raise NotImplementedError(
        "Live write_article not implemented. "
        "CrewAI will orchestrate this with GPT-4o-mini when OPENAI_API_KEY is set."
    )


def write_summary(content: str, max_words: int = 150) -> str:
    """
    Condense a block of text into a short summary.

    Args:
        content:   The text to summarise.
        max_words: Target word count for the summary.
    """
    if DEMO_MODE:
        words = content.split()
        truncated = " ".join(words[:max_words])
        if len(words) > max_words:
            truncated += "..."
        return json.dumps({
            "summary":    truncated,
            "word_count": min(len(words), max_words),
            "truncated":  len(words) > max_words,
        })
    raise NotImplementedError("Live write_summary not implemented.")


# ── Build the CrewAI agent ────────────────────────────────────────────────────

try:
    from crewai       import Agent as CrewAgent
    from crewai.tools import tool as crewai_tool
    _CREWAI_AVAILABLE = True
except ImportError:
    _CREWAI_AVAILABLE = False


if _CREWAI_AVAILABLE and not DEMO_MODE:
    @crewai_tool("write_article")
    def _crewai_write_article(topic: str, research: str = "", style: str = "informative") -> str:
        """Write a polished article from research findings."""
        return write_article(topic, research, style)

    @crewai_tool("write_summary")
    def _crewai_write_summary(content: str, max_words: int = 150) -> str:
        """Condense a block of text into a short summary."""
        return write_summary(content, max_words)

    _crew_agent = CrewAgent(
        role      = "Content Writer",
        goal      = "Produce high-quality, well-structured articles and summaries.",
        backstory  = (
            "You are an expert science and technology writer with 15 years of experience "
            "turning complex research into accessible, compelling content."
        ),
        tools     = [_crewai_write_article, _crewai_write_summary],
        verbose   = False,
    )
else:
    _crew_agent = None


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────

def build_writer_agent():
    """
    Build and return a Borgkit-wrapped WriterAgent.

    Returns a WrappedAgent (implements IAgent) ready for:
        await agent.register_discovery()
        await agent.handle_request(req)
    """
    _TOOLS = {
        "write_article": write_article,
        "write_summary": write_summary,
    }

    if DEMO_MODE or not _CREWAI_AVAILABLE:
        # Demo path: direct tool dispatch, no LLM needed.
        from interfaces.iagent          import IAgent
        from interfaces.agent_request   import AgentRequest
        from interfaces.agent_response  import AgentResponse
        from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
        from datetime import datetime, timezone

        class WriterAgentDemo(IAgent):
            agent_id = "borgkit://agent/writer"
            owner    = "0xWriterOwner"

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
                    name          = "WriterAgent",
                    owner         = self.owner,
                    capabilities  = self.get_capabilities(),
                    network       = NetworkInfo(protocol="http", host="localhost", port=8082),
                    health        = HealthStatus(status="healthy", last_heartbeat=now),
                    registered_at = now,
                    metadata_uri  = None,
                )

            async def register_discovery(self, discovery=None) -> None:
                from discovery.local_discovery import LocalDiscovery
                reg = discovery or LocalDiscovery.get_instance()
                await reg.register(self.get_anr())

        return WriterAgentDemo()

    else:
        # Live path: real CrewAIPlugin → GPT-4o-mini runs the crew
        from plugins.crewai_plugin import wrap_crewai
        return wrap_crewai(
            agent    = _crew_agent,
            name     = "WriterAgent",
            agent_id = "borgkit://agent/writer",
            owner    = "0xWriterOwner",
            tags     = ["writing", "crewai", "gpt4"],
            port     = 8082,
        )
