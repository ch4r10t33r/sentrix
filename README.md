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
| **L4** Execution | Agent frameworks | LangGraph · Google ADK · CrewAI · Agno · LlamaIndex · smolagents |
| **L3** Interaction | Request / response | `AgentRequest` / `AgentResponse` · AMP-2 |
| **L2** Discovery | Capability lookup | Local · HTTP · libp2p + Kademlia DHT · AMP-1 |
| **L1** Identity | DID + trust | `did:key` W3C (default) · ERC-8004 on-chain (optional) |

Sentrix operates primarily at **L2** and **L3**, bridging L1 identity to L4 framework execution.

---

## Features

- **Framework-agnostic** — wrap LangGraph, Google ADK, CrewAI, Agno, LlamaIndex, or smolagents agents with one function call
- **Dynamic discovery** — agents register capabilities; callers query at runtime, no hardcoded URLs
- **P2P mesh** — libp2p + QUIC + Kademlia DHT; mDNS for LAN; circuit relay for NAT traversal
- **DID identity** — `did:key` W3C standard out of the box; no wallet, no gas, no tokens required
- **x402 payments** — opt-in micropayment layer; charge per capability in USDC / ETH on Base
- **Multi-language** — TypeScript, Python, Rust, Zig
- **One CLI** — scaffold, create, run, discover

---

## Installation

```bash
npm install -g sentrix-cli
```

## Quick Start

```bash
# TypeScript (default)
sentrix init my-agent && cd my-agent && npm install && npm run dev

# Python
sentrix init my-agent --lang python

# Rust
sentrix init my-agent --lang rust

# Zig
sentrix init my-agent --lang zig
```

## Create agents (with framework support)

```bash
# Plain IAgent
sentrix create agent WeatherAgent -c get_weather,get_forecast

# LangGraph ReAct agent
sentrix create agent ResearchAgent -c web_search,summarise --framework langgraph

# Google ADK agent
sentrix create agent WriterAgent -c draft_section,format_report --framework google-adk

# Any of: google-adk | crewai | langgraph | agno | llamaindex | smolagents
sentrix create agent MyAgent -c capability1,capability2 --framework crewai

# With x402 payment add-on
sentrix create agent PaidAgent -c generate_image --framework langgraph --addon x402
```

---

## Generated Project Layout

```
my-agent/
├── interfaces/          # Core contracts (IAgent, AgentRequest, AgentResponse, IAgentDiscovery)
├── agents/              # Your agent implementations
├── discovery/           # Discovery adapters (Local, HTTP, libp2p)
├── identity/            # DID identity providers (no wallet required)
├── addons/
│   └── x402/            # x402 micropayment add-on (opt-in)
└── plugins/             # Framework adapters (LangGraph, ADK, CrewAI, …)
```

---

## Discovery Adapters

| Adapter | Backend | Use case |
|---|---|---|
| `LocalDiscovery` | In-memory | Dev & testing |
| `HttpDiscovery` | REST API | Centralised staging |
| `Libp2pDiscovery` | P2P / Kademlia DHT | Production mesh |
| `OnChainDiscovery` | ERC-8004 smart contract | On-chain registry (optional) |

---

## Identity — DID by default, no wallet needed

Sentrix uses **W3C Decentralized Identifiers (DID)** as the native identity format.

| Mode | DID format | How |
|------|-----------|-----|
| `local` (default) | `did:key:z...` | Key auto-created in `~/.sentrix/keystore/` |
| `env` | `did:key:z...` | `SENTRIX_AGENT_KEY=0x...` env var |
| `raw` | `did:key:z...` | Pass key directly (secret manager, HSM) |
| `erc8004` (optional) | `did:pkh:eip155:<chainId>:0x...` | On-chain wallet — adds verifiable ownership |

```python
# Python — no wallet, no gas, no registration required
from identity.provider import LocalKeystoreIdentity

identity = LocalKeystoreIdentity(name="my-agent")  # auto-creates key
print(identity.agent_id())  # did:key:zQ3shXXX...

config = PluginConfig(**identity.to_plugin_config_fields(), port=8080)
```

```typescript
// TypeScript
import { LocalKeystoreIdentity } from './identity';

const identity = new LocalKeystoreIdentity('my-agent');
console.log(identity.agentId());  // did:key:zQ3shXXX...
```

→ Full guide: **[docs/identity.md](docs/identity.md)**

---

## Cross-framework interoperability

A LangGraph agent and a Google ADK agent discovering and calling each other with zero framework coupling:

```python
# LangGraph agent on the mesh
from plugins.langgraph_plugin import wrap_langgraph
research_agent = wrap_langgraph(graph, name="ResearchAgent",
                                agent_id=identity.agent_id(), ...)

# Google ADK agent — discovers ResearchAgent at runtime
researchers = await registry.query('web_search')
peer = researchers[0]   # no idea it's LangGraph underneath
resp = await peer.handle_request(AgentRequest(capability='web_search', ...))
```

→ Full example: **[docs/examples/05-cross-framework.md](docs/examples/05-cross-framework.md)**

### Runnable example — Google ADK + CrewAI

[`examples/cross-framework/`](examples/cross-framework/) is a working end-to-end demo you can run right now:

```bash
git clone https://github.com/ch4r10t33r/sentrix
python3 examples/cross-framework/run.py
```

A **ResearchAgent** (Google ADK) and a **WriterAgent** (CrewAI) register with `LocalDiscovery`, then the orchestrator uses `AgentClient` to discover and call them in sequence — research findings flow from ADK into CrewAI without either agent knowing the other's framework. Runs in **demo mode by default** (no API keys needed); set `GOOGLE_API_KEY` and `OPENAI_API_KEY` to enable real LLMs.

---

## x402 Payments (opt-in)

Charge other agents for capabilities using the [x402 micropayment protocol](https://x402.org). Agents without pricing serve all requests free.

```python
from addons.x402 import X402ServerMixin, CapabilityPricing

class ImageGenAgent(X402ServerMixin, IAgent):
    x402_pricing = {
        "generate_image": CapabilityPricing.usdc_base(50, "0xMyWallet")  # $0.50 USDC
    }
    async def _handle_paid_request(self, req): ...
```

```typescript
import { withX402Payment, usdcBase } from './addons/x402';

const agent = withX402Payment(new MyAgent(), {
  pricing: { generate_image: usdcBase(50, '0xMyWallet') },
});
```

→ Full guide: **[docs/x402.md](docs/x402.md)**

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
| [docs/identity.md](docs/identity.md) | DID identity — all modes, ERC-8004 optional |
| [docs/x402.md](docs/x402.md) | x402 payment add-on |
| [docs/interfaces.md](docs/interfaces.md) | IAgent, AgentRequest, AgentResponse contracts |
| [docs/discovery.md](docs/discovery.md) | Discovery adapters |
| [docs/libp2p.md](docs/libp2p.md) | P2P networking with libp2p + QUIC |
| [docs/plugins.md](docs/plugins.md) | Framework adapters (6 frameworks) |
| [docs/differentiation.md](docs/differentiation.md) | How Sentrix differs from other frameworks |
| [docs/examples/](docs/examples/) | Worked examples (hello agent → cross-framework pipeline) |

---

## License

MIT
