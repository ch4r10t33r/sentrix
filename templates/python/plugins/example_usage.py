"""
Plugin Usage Examples
──────────────────────────────────────────────────────────────────────────────
Shows how to wrap LangGraph and Google ADK agents for Borgkit in <10 lines.

These are runnable examples — swap in your real agents and run with:
  python -m plugins.example_usage
"""

import asyncio


# ─────────────────────────────────────────────────────────────────────────────
# Example 1: LangGraph ReAct agent → Borgkit
# ─────────────────────────────────────────────────────────────────────────────

async def example_langgraph():
    """
    Wraps a standard LangGraph ReAct agent with two tools.
    Each tool becomes a separate Borgkit capability.
    """
    try:
        from langchain_core.tools import tool
        from langchain_openai import ChatOpenAI
        from langgraph.prebuilt import create_react_agent
    except ImportError:
        print("[skip] LangGraph not installed (pip install langgraph langchain-openai)")
        return

    from plugins.langgraph_plugin import wrap_langgraph

    # ── define tools ──────────────────────────────────────────────────────────
    @tool
    def get_weather(city: str) -> str:
        """Get current weather for a city."""
        return f"Sunny, 22°C in {city}"

    @tool
    def get_forecast(city: str, days: int = 3) -> str:
        """Get weather forecast for a city."""
        return f"{days}-day forecast for {city}: mostly cloudy"

    # ── build a LangGraph ReAct agent ─────────────────────────────────────────
    llm   = ChatOpenAI(model="gpt-4o-mini")
    graph = create_react_agent(llm, tools=[get_weather, get_forecast])

    # ── wrap for Borgkit (one function call) ───────────────────────────────────
    agent = wrap_langgraph(
        graph    = graph,
        name     = "WeatherAgent",
        agent_id = "borgkit://agent/weather",
        owner    = "0xYourWalletAddress",
        tags     = ["weather", "langraph"],
        tools    = [get_weather, get_forecast],   # explicit — skips graph introspection
    )

    print("Capabilities:", agent.get_capabilities())
    # → ['get_weather', 'get_forecast']

    # ── register on the Borgkit mesh ───────────────────────────────────────────
    await agent.register_discovery()

    # ── handle a Borgkit request ───────────────────────────────────────────────
    from interfaces import AgentRequest
    req  = AgentRequest(
        request_id = "req-001",
        from_id    = "0xCaller",
        capability = "get_weather",
        payload    = {"city": "London"},
    )
    resp = await agent.handle_request(req)
    print("Response:", resp.result)


# ─────────────────────────────────────────────────────────────────────────────
# Example 2: Google ADK agent → Borgkit
# ─────────────────────────────────────────────────────────────────────────────

async def example_google_adk():
    """
    Wraps a Google ADK agent with two FunctionTools.
    Each tool becomes a separate Borgkit capability.
    """
    try:
        from google.adk.agents import Agent
        from google.adk.tools import FunctionTool
    except ImportError:
        print("[skip] google-adk not installed (pip install google-adk)")
        return

    from plugins.google_adk_plugin import wrap_google_adk

    # ── define tools ──────────────────────────────────────────────────────────
    def search_docs(query: str) -> str:
        """Search the documentation knowledge base."""
        return f"Found 3 articles for '{query}'"

    def create_ticket(title: str, priority: str = "medium") -> str:
        """Create a support ticket."""
        return f"Ticket created: {title} (priority: {priority})"

    # ── build a Google ADK agent ──────────────────────────────────────────────
    adk_agent = Agent(
        name        = "support_agent",
        model       = "gemini-2.0-flash",
        description = "A support agent that can search docs and create tickets",
        tools       = [FunctionTool(search_docs), FunctionTool(create_ticket)],
    )

    # ── wrap for Borgkit (one function call) ───────────────────────────────────
    agent = wrap_google_adk(
        agent    = adk_agent,
        name     = "SupportAgent",
        agent_id = "borgkit://agent/support",
        owner    = "0xYourWalletAddress",
        tags     = ["support", "helpdesk", "adk"],
    )

    print("Capabilities:", agent.get_capabilities())
    # → ['search_docs', 'create_ticket']

    # ── register on the Borgkit mesh ───────────────────────────────────────────
    await agent.register_discovery()

    # ── handle a Borgkit request ───────────────────────────────────────────────
    from interfaces import AgentRequest
    req  = AgentRequest(
        request_id = "req-002",
        from_id    = "0xCaller",
        capability = "search_docs",
        payload    = {"query": "how to reset password"},
    )
    resp = await agent.handle_request(req)
    print("Response:", resp.result)


# ─────────────────────────────────────────────────────────────────────────────
# Example 3: Multi-framework mesh
# ─────────────────────────────────────────────────────────────────────────────

async def example_multi_framework():
    """
    A LangGraph agent and a Google ADK agent coexist on the same Borgkit mesh.
    Agents can discover and call each other using standard Borgkit requests,
    regardless of their underlying framework.
    """
    from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
    from discovery.local_discovery import LocalDiscovery
    from datetime import datetime, timezone

    registry = LocalDiscovery.get_instance()

    # Simulate both agents registering
    for agent_id, name, caps in [
        ("borgkit://agent/weather", "WeatherAgent", ["get_weather", "get_forecast"]),
        ("borgkit://agent/support", "SupportAgent", ["search_docs", "create_ticket"]),
    ]:
        await registry.register(DiscoveryEntry(
            agent_id=agent_id,
            name=name,
            owner="0xWallet",
            capabilities=caps,
            network=NetworkInfo(protocol="http", host="localhost", port=6174),
            health=HealthStatus(status="healthy", last_heartbeat=datetime.now(timezone.utc).isoformat()),
            registered_at=datetime.now(timezone.utc).isoformat(),
        ))

    # Any Borgkit agent can now discover the others
    weather_agents = await registry.query("get_weather")
    support_agents = await registry.query("search_docs")

    print("\n=== Mesh Discovery ===")
    print("Weather agents:", [a.agent_id for a in weather_agents])
    print("Support agents:", [a.agent_id for a in support_agents])
    print("All agents on mesh:", [a.agent_id for a in await registry.list_all()])


if __name__ == "__main__":
    asyncio.run(example_multi_framework())   # works without any ML deps
    # asyncio.run(example_langgraph())       # requires LangGraph + OpenAI key
    # asyncio.run(example_google_adk())      # requires google-adk + Gemini key
