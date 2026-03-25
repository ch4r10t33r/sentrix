# Sentrix

> **Autonomous Agentic Coordination Middleware** — scaffold P2P-discoverable, DID-native AI agents that interoperate across any framework, with optional ERC-8004 on-chain compliance.

Like TCP/IP connects heterogeneous computers, Sentrix connects heterogeneous agents.

---

## Why Sentrix?

Most AI frameworks help you **build** agents. Sentrix helps them **find and talk to each other** — across frameworks, runtimes, and clouds.

| What exists today | The gap Sentrix fills |
|---|---|
| Orchestration frameworks (CrewAI, AutoGen) | Agents locked inside one runtime, no external discovery |
| Framework-bound agents (LangGraph, AutoGPT) | No standardised interface for cross-framework calls |
| Closed ecosystems (Fetch.ai, SingularityNET) | Ecosystem lock-in, heavyweight infrastructure |

Sentrix is **not a platform** — it is a protocol layer others build on.

---

## Architecture

| Layer | Role | Technologies |
|---|---|---|
| **L4** Execution | Agent frameworks | LangGraph · Google ADK · CrewAI · Agno · LlamaIndex · smolagents · OpenAI Agents |
| **L3** Interaction | Request / response | `AgentRequest` / `AgentResponse` · AMP-2 |
| **L2** Discovery | Capability lookup | Local · HTTP · libp2p + Kademlia DHT · AMP-1 |
| **L1** Identity | DID + trust | `did:key` W3C (default) · ERC-8004 on-chain (optional) |

Sentrix operates primarily at **L2** and **L3**, bridging L1 identity to L4 framework execution.

---

## Features

- **Framework-agnostic** — wrap LangGraph, Google ADK, CrewAI, Agno, LlamaIndex, smolagents, or OpenAI Agents with one function call
- **Built-in HTTP server** — `sentrix run MyAgent --port 8080` starts a real HTTP server; no extra setup
- **MCP bridge** — any MCP server becomes a Sentrix agent; any Sentrix agent becomes an MCP server (Claude Desktop, Cursor, Continue)
- **Dynamic discovery** — agents register capabilities; callers query at runtime, no hardcoded URLs
- **Mesh protocols** — heartbeat, capability exchange (as part of handshake), and gossip fan-out built in
- **P2P mesh** — libp2p + QUIC + Kademlia DHT; mDNS for LAN; circuit relay for NAT traversal
- **DID identity** — `did:key` W3C standard out of the box; no wallet, no gas, no tokens required
- **x402 payments** — opt-in micropayment layer; charge per capability in USDC / ETH on Base
- **Multi-language** — TypeScript, Python, Rust, Zig
- **One CLI** — scaffold, create, run, discover

---

## Installation

```bash
npm install -g @ch4r01teer41/sentrix-cli
```

---

## Quick Start

```bash
# Scaffold a new project (TypeScript default)
sentrix init my-agent
cd my-agent && npm install
sentrix run ExampleAgent --port 8080

# Python
sentrix init my-agent --lang python
cd my-agent
sentrix run ExampleAgent --port 8080

# Rust
sentrix init my-agent --lang rust

# Zig
sentrix init my-agent --lang zig
```

Once running, the agent prints its full startup banner:

```
────────────────────────────────────────────────────────────
  Sentrix Agent Online  v0.1.0
────────────────────────────────────────────────────────────
  Name         ExampleAgent
  Agent ID     sentrix://agent/example
  Endpoint     http://0.0.0.0:8080
  Discovery    local
  Capabilities (2)
           • echo
           • ping
────────────────────────────────────────────────────────────
```

---

## CLI Reference

| Command | Description |
|---|---|
| `sentrix init <name> [--lang ts\|python\|rust\|zig]` | Scaffold a new Sentrix project |
| `sentrix create agent <name> [-c cap1,cap2] [--framework X]` | Add an agent to an existing project |
| `sentrix run <AgentName> [--port 8080]` | Start an agent's HTTP server |
| `sentrix discover [-c capability] [--host h] [--port p]` | Query the discovery layer |
| `sentrix version` | Show CLI version and build info |

### `sentrix run` — HTTP endpoints

When you run an agent, these endpoints are live automatically:

| Endpoint | Method | Description |
|---|---|---|
| `/invoke` | `POST` | Call any capability — `{ capability, payload, from }` → `AgentResponse` |
| `/health` | `GET` | Heartbeat — returns health status, capability count, version |
| `/anr` | `GET` | Full Agent Network Record (ANR) as JSON |
| `/capabilities` | `GET` | List of capability names |
| `/gossip` | `POST` | Receive gossip messages from mesh peers |

---

## Framework Plugins

### Python — all supported frameworks

```python
# Google ADK
from plugins.google_adk_plugin import wrap_google_adk
agent = wrap_google_adk(adk_agent, name="SupportBot", agent_id="sentrix://agent/support", owner="0x...")

# CrewAI
from plugins.crewai_plugin import wrap_crewai
agent = wrap_crewai(crew_agent, name="ResearchBot", agent_id="sentrix://agent/research", owner="0x...")

# LangGraph
from plugins.langgraph_plugin import wrap_langgraph
agent = wrap_langgraph(compiled_graph, config)

# OpenAI Agents SDK
from plugins.openai_plugin import wrap_openai
agent = wrap_openai(oai_agent, name="WeatherBot", agent_id="sentrix://agent/weather", owner="0x...")

# Agno / LlamaIndex / smolagents
from plugins.agno_plugin       import wrap_agno
from plugins.llamaindex_plugin import wrap_llamaindex
from plugins.smolagents_plugin import wrap_smolagents

# Serve over HTTP (all plugins share the same interface)
import asyncio
asyncio.run(agent.serve(port=8080))
```

### TypeScript

```typescript
// LangGraph
import { wrapLangGraph }  from './plugins/LangGraphPlugin';
const agent = wrapLangGraph(compiledGraph, config);

// OpenAI Agents SDK
import { wrapOpenAI } from './plugins/OpenAIPlugin';
const agent = wrapOpenAI(oaiAgent, { agentId: 'sentrix://agent/weather', name: 'WeatherBot', ... });

await agent.serve({ port: 8080 });
```

---

## MCP Bridge

Sentrix has a two-way bridge with the [Model Context Protocol](https://modelcontextprotocol.io):

```python
# Any MCP server → Sentrix agent (GitHub, filesystem, Slack, databases…)
from plugins.mcp_plugin import MCPPlugin
plugin = await MCPPlugin.from_command(
    ["npx", "-y", "@modelcontextprotocol/server-github"],
    config, env={"GITHUB_TOKEN": "ghp_..."}
)
agent = plugin.wrap()
await agent.serve(port=8081)

# Any Sentrix agent → MCP server (Claude Desktop, Cursor, Continue…)
from adapters.mcp_server import serve_as_mcp
await serve_as_mcp(my_agent)                           # stdio (Claude Desktop)
await serve_as_mcp(my_agent, transport="sse", port=3000)  # SSE (remote)
```

```typescript
// TypeScript — same bridge
import { MCPPlugin }    from './plugins/MCPPlugin';
import { serveAsMcp }  from './adapters/MCPServer';

const plugin = await MCPPlugin.fromCommand(['npx', '-y', '@modelcontextprotocol/server-github'], config);
await plugin.wrap().serve({ port: 8081 });

await serveAsMcp(myAgent);                              // stdio
await serveAsMcp(myAgent, { transport: 'sse', port: 3000 });
```

→ Full guide: **[docs/mcp.md](docs/mcp.md)**

---

## Agent-to-Agent Communication

```python
from interfaces.iagent_client import AgentClient
from discovery.local_discovery import LocalDiscovery

client = AgentClient(discovery=LocalDiscovery.get_instance())

# 1. Find an agent by capability
entry = await client.find("web_search")

# 2. Connect (handshake: heartbeat + capability exchange) → AgentSession
session = await client.connect(entry)
print(session.capabilities)    # ['web_search', 'summarise']
print(session.is_healthy)      # True

# 3. Call via the session
resp = await session.call("web_search", {"query": "latest AI news"})
print(resp.result["content"])

# Or skip the handshake for one-off calls
resp = await client.call_capability("web_search", {"query": "latest AI news"})
```

→ Full guide: **[docs/interfaces.md](docs/interfaces.md)**

---

## Discovery Adapters

| Adapter | Backend | Use case |
|---|---|---|
| `LocalDiscovery` | In-memory | Dev & testing |
| `HttpDiscovery` | REST API | Centralised staging |
| `GossipDiscovery` | HTTP fan-out + TTL | Decentralised mesh (no DHT required) |
| `Libp2pDiscovery` | P2P / Kademlia DHT | Production mesh |
| `OnChainDiscovery` | ERC-8004 smart contract | On-chain registry (optional) |

---

## Identity — DID by default, no wallet needed

| Mode | DID format | How |
|------|-----------|-----|
| `local` (default) | `did:key:z...` | Key auto-created in `~/.sentrix/keystore/` |
| `env` | `did:key:z...` | `SENTRIX_AGENT_KEY=0x...` env var |
| `raw` | `did:key:z...` | Pass key directly (secret manager, HSM) |
| `erc8004` (optional) | `did:pkh:eip155:<chainId>:0x...` | On-chain wallet — adds verifiable ownership |

```python
from identity.provider import LocalKeystoreIdentity

identity = LocalKeystoreIdentity(name="my-agent")
print(identity.agent_id())  # did:key:zQ3shXXX...

config = PluginConfig(**identity.to_plugin_config_fields(), port=8080)
```

→ Full guide: **[docs/identity.md](docs/identity.md)**

---

## x402 Payments (opt-in)

Charge other agents per capability in USDC on Base. Agents without pricing serve all requests free.

```python
from addons.x402.types import CapabilityPricing

config = PluginConfig(
    ...
    x402_pricing={
        "generate_image": CapabilityPricing.usdc_base(0.05, "0xYourWallet"),  # $0.05 per call
    }
)
```

The HTTP server gate is automatic — no code needed in the agent. Callers receive an HTTP 402 with the full payment challenge if they haven't included a proof.

→ Full guide: **[docs/x402.md](docs/x402.md)**

### Runnable example — Google ADK + CrewAI

[`examples/cross-framework/`](examples/cross-framework/) is a working end-to-end demo you can run right now:

```bash
git clone https://github.com/ch4r10t33r/sentrix
python3 examples/cross-framework/run.py
```

A **ResearchAgent** (Google ADK) and a **WriterAgent** (CrewAI) register with `LocalDiscovery`, then the orchestrator uses `AgentClient` to discover and call them in sequence — research findings flow from ADK into CrewAI without either agent knowing the other's framework. Runs in **demo mode by default** (no API keys needed); set `GOOGLE_API_KEY` and `OPENAI_API_KEY` to enable real LLMs.

---

## Cross-Framework Example

A Google ADK research agent and a CrewAI writer agent discovering and calling each other with zero framework coupling:

```bash
cd examples/cross-framework
pip install -r requirements.txt
python run_example.py
```

→ **[examples/cross-framework/](examples/cross-framework/)**

---

## AMP Specification Modules

| Module | Description | Status |
|---|---|---|
| AMP-1 | Discovery (capability indexing + queries) | ✅ Stable |
| AMP-2 | Interaction (request/response + routing) | ✅ Stable |
| AMP-3 | Payments (x402 micropayments) | ✅ Add-on available |
| AMP-4 | Delegation & multi-agent workflows | 🔜 Roadmap |

---

## Documentation

| Doc | Description |
|---|---|
| [docs/overview.md](docs/overview.md) | Architecture and core concepts |
| [docs/interfaces.md](docs/interfaces.md) | IAgent, AgentSession, IAgentClient — full interface reference |
| [docs/identity.md](docs/identity.md) | DID identity — all modes, ERC-8004 optional |
| [docs/x402.md](docs/x402.md) | x402 payment add-on |
| [docs/mcp.md](docs/mcp.md) | MCP bridge — wrap MCP servers, expose as MCP server |
| [docs/discovery.md](docs/discovery.md) | Discovery adapters |
| [docs/libp2p.md](docs/libp2p.md) | P2P networking with libp2p + QUIC |
| [docs/plugins.md](docs/plugins.md) | Framework adapters (7 frameworks) |
| [docs/differentiation.md](docs/differentiation.md) | How Sentrix differs from other frameworks |

---

## TODOs

- [ ] More examples, tutorials and videos
- [ ] Rust + Zig mesh protocol implementations
- [ ] `sentrix test` — unit-test framework for agents
- [ ] `sentrix inspect <endpoint>` — CLI ANR + capability inspector
- [ ] Streaming responses (SSE / WebSocket)
- [ ] Public hosted discovery registry

---

## License

Apache 2.0 — see [LICENSE](LICENSE)
