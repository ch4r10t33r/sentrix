# Cross-Framework A2A Example

Two agents built with different frameworks discover and call each other via the Borgkit mesh.

| Agent | Framework | Capabilities |
|---|---|---|
| **ResearchAgent** | Google ADK + Gemini 2.0 Flash | `research_topic`, `find_recent_papers` |
| **WriterAgent** | CrewAI + GPT-4o-mini | `write_article`, `write_summary` |

**Pipeline:** `research_topic` → results passed to → `write_article` → final article

---

## Run it (no API keys needed)

```bash
cd examples/cross-framework
python run.py
```

Both agents run in **demo mode** by default — tool functions return realistic mock data, no LLM calls are made. The full Borgkit plumbing runs: capability extraction, discovery registration, `AgentClient` lookup, and A2A request/response.

Change the topic:
```bash
RESEARCH_TOPIC="quantum computing" python run.py
```

---

## Run with real LLMs

```bash
cp .env.example .env
# edit .env with your keys

# Install framework dependencies
pip install google-adk google-generativeai      # for ResearchAgent
pip install crewai crewai-tools openai          # for WriterAgent

export GOOGLE_API_KEY=your-key
export OPENAI_API_KEY=your-key
python run.py
```

In live mode:
- **ResearchAgent**: Gemini 2.0 Flash decides when to call `research_topic` / `find_recent_papers` based on the request
- **WriterAgent**: CrewAI runs a crew with GPT-4o-mini to complete `write_article` / `write_summary` tasks

---

## What this demonstrates

```
┌─────────────────────────────────────────────────────────────┐
│  LocalDiscovery  (shared in-memory registry)                │
│                                                             │
│  ResearchAgent  ──registers──►  {research_topic, ...}       │
│  WriterAgent    ──registers──►  {write_article,  ...}       │
└─────────────────────────────────────────────────────────────┘
                           │
          AgentClient.find("research_topic")
                           │
                           ▼
          ┌────────────────────────────┐
          │  ResearchAgent (ADK)       │
          │  handle_request(req)       │
          │  → research_topic(topic)   │
          └────────────────────────────┘
                           │
                     AgentResponse
                           │
          AgentClient.find("write_article")
                           │
                           ▼
          ┌────────────────────────────┐
          │  WriterAgent (CrewAI)      │
          │  handle_request(req)       │
          │  → write_article(topic,    │
          │       research=<above>)    │
          └────────────────────────────┘
                           │
                     Final Article
```

### Key Borgkit concepts shown

- **Plugin wrapping** — `wrap_google_adk()` and `wrap_crewai()` adapt framework agents to `IAgent` in one call
- **Capability extraction** — tools (`FunctionTool`, `@crewai_tool`) automatically become Borgkit capabilities
- **Discovery** — both agents register with `LocalDiscovery`; swap for `HttpDiscovery` or `GossipDiscovery` in production
- **AgentClient** — `find(capability)` → `call_capability(...)` works the same regardless of underlying framework
- **AgentRequest / AgentResponse** — the universal message envelope crossing framework boundaries

---

## Project structure

```
examples/cross-framework/
├── research_agent.py   Google ADK agent definition + Borgkit wrapping
├── writer_agent.py     CrewAI agent definition + Borgkit wrapping
├── run.py              Orchestrator: registers agents, runs the pipeline
├── requirements.txt    Framework dependencies (commented out for demo mode)
└── .env.example        API key template
```

---

## Extending this example

**Add a third agent** (e.g. a fact-checker using LangGraph):
```bash
cd /path/to/your/borgkit-project
borgkit create agent FactCheckerAgent -c fact_check,find_sources --framework langgraph
```

**Add x402 micropayment** to the writer agent:
```python
from addons.x402.server import with_x402_payment
from addons.x402.types  import usdc_base

writer = with_x402_payment(build_writer_agent(), pricing={
    "write_article": usdc_base(10, "0xYourWallet"),  # $0.10 per article
})
```
