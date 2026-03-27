"""
Cross-framework A2A example
────────────────────────────────────────────────────────────────────────────────
Two agents built with different frameworks discover and call each other via the
Borgkit mesh:

  ResearchAgent  (Google ADK / Gemini)   — capability: research_topic
  WriterAgent    (CrewAI    / GPT-4o)    — capability: write_article

Pipeline:
  1. Both agents register with LocalDiscovery.
  2. The orchestrator uses AgentClient to find the ResearchAgent by capability
     and calls research_topic("AI agents in healthcare").
  3. The research result is forwarded to WriterAgent.write_article().
  4. The finished article is printed.

Running modes
─────────────
  Demo mode  (default, no API keys needed):
    python run.py

  Live mode  (uses real LLMs):
    export GOOGLE_API_KEY=...   # enables Gemini via Google ADK
    export OPENAI_API_KEY=...   # enables GPT-4o-mini via CrewAI
    python run.py
"""
from __future__ import annotations
import asyncio, json, os, sys, textwrap, time
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../../templates/python"))

from research_agent import build_research_agent, DEMO_MODE as ADK_DEMO
from writer_agent   import build_writer_agent,   DEMO_MODE as CREW_DEMO

from discovery.local_discovery   import LocalDiscovery
from interfaces.iagent_client    import AgentClient
from interfaces.agent_request    import AgentRequest

# ── Colours ───────────────────────────────────────────────────────────────────
R  = "\033[0m";  B  = "\033[1m";  C  = "\033[36m"
G  = "\033[32m"; Y  = "\033[33m"; M  = "\033[35m"
DIM = "\033[2m";  LINE = f"{DIM}{'─' * 68}{R}"


def banner():
    mode_adk  = f"{Y}DEMO{R}"  if ADK_DEMO  else f"{G}LIVE (Gemini){R}"
    mode_crew = f"{Y}DEMO{R}"  if CREW_DEMO else f"{G}LIVE (GPT-4o){R}"
    print(f"""
{LINE}
  {B}{C}Borgkit Cross-Framework Example{R}
{LINE}
  {B}ResearchAgent{R}  Google ADK   [{mode_adk}]
  {B}WriterAgent{R}    CrewAI       [{mode_crew}]
{LINE}
  Pipeline:  research_topic  →  write_article
{LINE}
""")


def section(title: str):
    print(f"\n{B}{C}▶ {title}{R}")
    print(f"{DIM}{'─' * 50}{R}")


def print_response(label: str, resp):
    status_color = G if resp.status == "success" else Y
    print(f"  {B}Status{R}  {status_color}{resp.status}{R}")
    if resp.status == "success" and resp.result:
        content = resp.result.get("content", resp.result)
        if isinstance(content, dict):
            for k, v in content.items():
                if isinstance(v, list):
                    print(f"  {B}{k}{R}")
                    for item in v:
                        print(f"    {G}•{R} {item}")
                elif isinstance(v, str) and len(v) > 80:
                    print(f"  {B}{k}{R}")
                    for line in textwrap.wrap(v, 64):
                        print(f"    {line}")
                else:
                    print(f"  {B}{k}{R}  {v}")
        else:
            for line in textwrap.wrap(str(content), 66):
                print(f"  {line}")
    elif resp.error_message:
        print(f"  {Y}{resp.error_message}{R}")


# ── In-process client ─────────────────────────────────────────────────────────
#
# Borgkit AgentClient normally dispatches over HTTP (POST /invoke).
# For this single-process demo we route calls directly to the in-memory
# agent instances — same API, no network required.
#
# In production: replace with  AgentClient(discovery)  and start each agent
# as a separate HTTP server with  borgkit run --port <N>

class InProcessClient:
    """
    Routes AgentRequests directly to registered in-process agent instances.
    Mirrors the AgentClient.call_capability / call API exactly.
    In production, swap this for AgentClient(discovery).
    """
    def __init__(self, discovery: LocalDiscovery, agents: dict):
        self._discovery = discovery
        self._agents    = agents   # {agent_id: IAgent}
        self._client    = AgentClient(discovery)

    async def call_capability(self, capability: str, payload: dict, *, caller_id: str = "orchestrator"):
        entry = await self._client.find(capability)
        if entry is None:
            from interfaces.agent_response import AgentResponse
            return AgentResponse.error("n/a", f"No agent found for capability: '{capability}'")
        agent = self._agents.get(entry.agent_id)
        if agent is None:
            from interfaces.agent_response import AgentResponse
            return AgentResponse.error("n/a", f"Agent {entry.agent_id} not reachable in-process")
        import uuid
        req = AgentRequest(
            request_id = str(uuid.uuid4()),
            from_id    = caller_id,
            capability = capability,
            payload    = payload,
        )
        return await agent.handle_request(req)

    async def find(self, capability: str):
        return await self._client.find(capability)


# ── Main ──────────────────────────────────────────────────────────────────────

async def main():
    banner()
    topic = os.getenv("RESEARCH_TOPIC", "AI agents in healthcare")

    # ── 1. Build agents ───────────────────────────────────────────────────────
    section("Building agents")

    research_agent = build_research_agent()
    writer_agent   = build_writer_agent()

    print(f"  {G}✔{R} ResearchAgent  capabilities: {research_agent.get_capabilities()}")
    print(f"  {G}✔{R} WriterAgent    capabilities: {writer_agent.get_capabilities()}")

    # ── 2. Register with discovery ────────────────────────────────────────────
    section("Registering with LocalDiscovery")

    discovery = LocalDiscovery.get_instance()
    await research_agent.register_discovery(discovery)
    await writer_agent.register_discovery(discovery)

    all_entries = await discovery.list_all()
    print(f"  {G}✔{R} {len(all_entries)} agents on the mesh:")
    for e in all_entries:
        print(f"       {DIM}{e.agent_id}{R}  →  {e.capabilities}")

    # ── 3. Create the in-process client ──────────────────────────────────────
    client = InProcessClient(discovery, {
        research_agent.agent_id: research_agent,
        writer_agent.agent_id:   writer_agent,
    })

    # ── 4. Discover the research agent ───────────────────────────────────────
    section("Step 1 — Discover: find agent for 'research_topic'")

    entry = await client.find("research_topic")
    if entry is None:
        print(f"  {Y}No agent found for 'research_topic' — aborting.{R}")
        return

    print(f"  {G}✔{R} Found: {B}{entry.name}{R} ({entry.agent_id})")
    print(f"       Endpoint : {entry.network.protocol}://{entry.network.host}:{entry.network.port}")
    print(f"       Capabilities: {entry.capabilities}")

    # ── 5. Call research_topic ────────────────────────────────────────────────
    section(f"Step 2 — Call: research_topic(\"{topic}\")")

    t0 = time.monotonic()
    research_resp = await client.call_capability(
        "research_topic",
        {"topic": topic, "depth": "standard"},
    )
    elapsed = int((time.monotonic() - t0) * 1000)

    print_response("research_topic", research_resp)
    print(f"\n  {DIM}↳ {elapsed}ms{R}")

    if research_resp.status != "success":
        print(f"\n  {Y}Research step failed — skipping write step.{R}")
        return

    # ── 6. Forward research → WriterAgent ─────────────────────────────────────
    section(f"Step 3 — Call: write_article(\"{topic}\")")

    research_content = research_resp.result or {}
    t0 = time.monotonic()
    article_resp = await client.call_capability(
        "write_article",
        {
            "topic":    topic,
            "research": json.dumps(research_content),
            "style":    "informative",
        },
    )
    elapsed = int((time.monotonic() - t0) * 1000)

    print_response("write_article", article_resp)
    print(f"\n  {DIM}↳ {elapsed}ms{R}")

    # ── 7. Print the final article ────────────────────────────────────────────
    if article_resp.status == "success":
        content = article_resp.result.get("content", {})
        article_text = content.get("article", "") if isinstance(content, dict) else str(content)
        if article_text:
            print(f"\n{LINE}")
            print(f"  {B}{C}Final Article{R}")
            print(f"{LINE}")
            for line in article_text.splitlines():
                print(f"  {line}")
            print(f"{LINE}\n")

    # ── 8. Summary ────────────────────────────────────────────────────────────
    section("Done")
    mode = f"{Y}DEMO{R} (no API keys)" if (ADK_DEMO or CREW_DEMO) else f"{G}LIVE{R}"
    print(f"  Mode     : {mode}")
    print(f"  Topic    : {topic}")
    print(f"  Agents   : {len(all_entries)} registered on mesh")
    print(f"  Pipeline : research_topic → write_article")
    print()
    if ADK_DEMO or CREW_DEMO:
        print(f"  {DIM}To run with real LLMs:{R}")
        print(f"  {DIM}  export GOOGLE_API_KEY=<your-key>   # enables Gemini via ADK{R}")
        print(f"  {DIM}  export OPENAI_API_KEY=<your-key>   # enables GPT-4o via CrewAI{R}")
        print(f"  {DIM}  python run.py{R}")
    print()


if __name__ == "__main__":
    asyncio.run(main())
