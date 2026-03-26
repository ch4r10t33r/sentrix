<p align="center">
  <img src="https://raw.githubusercontent.com/ch4r10t33r/sentrix/main/docs/logo.svg" alt="Sentrix" width="220"/>
</p>

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
- **Built-in HTTP server** — `sentrix run MyAgent --port 6174` starts a real HTTP server; no extra setup
- **MCP bridge** — any MCP server becomes a Sentrix agent; any Sentrix agent becomes an MCP server (Claude Desktop, Cursor, Continue)
- **Dynamic discovery** — agents register capabilities; callers query at runtime, no hardcoded URLs
- **Mesh protocols** — heartbeat, capability exchange (as part of handshake), and gossip fan-out built in
- **P2P mesh** — libp2p + QUIC + Kademlia DHT; mDNS for LAN; circuit relay for NAT traversal
- **DID identity** — `did:key` W3C standard out of the box; no wallet, no gas, no tokens required
- **x402 payments** — opt-in micropayment layer; charge per capability in USDC / ETH on Base
- **Multi-language** — TypeScript, Python, Rust, Zig
- **One CLI** — scaffold, create, run, discover

---

## Language Coverage

| Feature | Python | TypeScript | Rust | Zig |
|---|:---:|:---:|:---:|:---:|
| **IAgent interface** | ✅ | ✅ | ✅ | ✅ |
| **AgentRequest / AgentResponse** | ✅ | ✅ | ✅ | ✅ |
| **ANR (Agent Network Record)** | ✅ | ✅ | ✅ | ✅ |
| **DID identity (`did:key`)** | ✅ | ✅ | 🔜 | 🔜 |
| **HTTP server (`sentrix run`)** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — local (in-memory)** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — HTTP** | ✅ | ✅ | ✅ | 🔜 |
| **Discovery — libp2p + Kademlia DHT** | ✅ | ✅ | ✅ | 🔜 |
| **Discovery — gossip fan-out** | ✅ | ✅ | 🔜 | 🔜 |
| **AgentClient (mesh protocols)** | ✅ | ✅ | ✅ | ✅ |
| **Example agent** | ✅ | ✅ | ✅ | ✅ |
| **Plugin system (framework adapters)** | ✅ | ✅ | ✅ | ✅ |
| **LangGraph plugin** | ✅ | ✅ | — | — |
| **Google ADK plugin** | ✅ | 🔜 | — | — |
| **CrewAI plugin** | ✅ | — | — | — |
| **OpenAI Agents SDK plugin** | ✅ | ✅ | — | — |
| **Agno plugin** | ✅ | 🔜 | — | — |
| **LlamaIndex plugin** | ✅ | 🔜 | — | — |
| **smolagents plugin** | ✅ | 🔜 | — | — |
| **MCP bridge (wrap MCP servers)** | ✅ | ✅ | — | — |
| **MCP bridge (expose as MCP server)** | ✅ | ✅ | — | — |
| **x402 micropayments** | ✅ | ✅ | 🔜 | 🔜 |
| **Streaming (SSE via /invoke/stream)** | ✅ | ✅ | 🔜 | 🔜 |

**Legend:** ✅ implemented · 🔜 on roadmap · — not applicable for this language

---

## Installation

### npm (recommended)

Works on macOS, Linux, and Windows. npm downloads the correct pre-built
binary for your platform automatically.

```bash
npm install -g @ch4r10teer41/sentrix-cli
```

### curl installer (macOS / Linux)

Auto-detects your OS and architecture, installs to `/usr/local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/sentrix/main/install.sh | sh
```

To install to a custom directory set `SENTRIX_INSTALL_DIR` before piping:

```bash
SENTRIX_INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/sentrix/main/install.sh | sh
```

> **Windows** — use npm above, or download `sentrix-win32-x64.exe` directly from the
> [Releases page](https://github.com/ch4r10t33r/sentrix/releases/latest).

### Build from source

Requires [Rust](https://rustup.rs) 1.75+.

```bash
git clone https://github.com/ch4r10t33r/sentrix.git
cd sentrix
cargo build --release --package sentrix-cli
# Binary is at ./target/release/sentrix
```

---

## Quick Start

```bash
# Scaffold a new project (TypeScript default)
sentrix init my-agent
cd my-agent && npm install
sentrix run ExampleAgent --port 6174

# Python
sentrix init my-agent --lang python
cd my-agent
sentrix run ExampleAgent --port 6174

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
  Endpoint     http://0.0.0.0:6174
  Multiaddr    /ip4/0.0.0.0/tcp/6174/p2p/12D3Koo...  (libp2p mode)
  Discovery    local
  Capabilities (2)
           • echo
           • ping
────────────────────────────────────────────────────────────
```

> **Default port: 6174** ([Kaprekar's constant](https://en.wikipedia.org/wiki/6174)). Override with `SENTRIX_PORT=<n>` or `--port <n>`.

---

## CLI Reference

| Command | Description |
|---|---|
| `sentrix init <name> [--lang ts\|python\|rust\|zig]` | Scaffold a new Sentrix project |
| `sentrix create agent <name> [-c cap1,cap2] [--framework X]` | Add an agent to an existing project |
| `sentrix run <AgentName> [--port 6174]` | Start an agent's HTTP server |
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
asyncio.run(agent.serve(port=6174))
```

### TypeScript

```typescript
// LangGraph
import { wrapLangGraph }  from './plugins/LangGraphPlugin';
const agent = wrapLangGraph(compiledGraph, config);

// OpenAI Agents SDK
import { wrapOpenAI } from './plugins/OpenAIPlugin';
const agent = wrapOpenAI(oaiAgent, { agentId: 'sentrix://agent/weather', name: 'WeatherBot', ... });

await agent.serve({ port: 6174 });
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

## Protocol Reference

### Ports and Addresses

| Layer | Default address | Env override |
|---|---|---|
| HTTP server (`/invoke`, `/health`, …) | `0.0.0.0:6174` | `SENTRIX_PORT` |
| libp2p TCP (GossipSub + request-response) | `/ip4/0.0.0.0/tcp/6174` | `SENTRIX_P2P_ADDR` |
| libp2p QUIC (Rust/Zig DHT) | `/ip4/0.0.0.0/udp/6174/quic-v1` | `SENTRIX_P2P_PORT` |
| Bootstrap peers | _(none — mDNS only on LAN)_ | `SENTRIX_BOOTSTRAP_PEERS` (comma-separated multiaddrs) |

All discovery traffic — DHT announces, gossip fan-out, and capability queries — travels **over the libp2p transport on the same port**. There is no separate discovery port.

Every agent's ANR always carries the full `multiaddr` when running in libp2p mode:

```
/ip4/<host>/tcp/<port>/p2p/<PeerId>          # TCP (TypeScript)
/ip4/<host>/udp/<port>/quic-v1/p2p/<PeerId>  # QUIC (Rust)
```

When only HTTP transport is active, `network.multiaddr` is empty and `network.protocol` is `"http"`.

---

### Agent DID and Identity

Every agent has a **DID** derived from its secp256k1 keypair — no wallet or gas required.

| Mode | DID format | `agentId` |
|---|---|---|
| `local` (default) | `did:key:zQ3sh…` | `sentrix://agent/<eth-addr>` |
| `env` | `did:key:zQ3sh…` | same, driven by `SENTRIX_AGENT_KEY` |
| `erc8004` | `did:pkh:eip155:<chainId>:0x…` | on-chain verified owner |

The DID is a multicodec-prefixed, base58btc-encoded secp256k1 compressed public key:

```
did:key:z  <base58btc( 0xe701 || compressed-secp256k1-pubkey )>
            ↑ secp256k1 multicodec varint
```

### Agent Network Record (ANR)

The ANR is the canonical self-description of an agent. It is returned by `GET /anr`, stored in the Kademlia DHT, and broadcast via gossip. Its shape is `DiscoveryEntry`:

```typescript
interface DiscoveryEntry {
  agentId:      string;           // "sentrix://agent/0xABC…" or a DID
  name:         string;           // human-readable label
  owner:        string;           // Ethereum address or DID of the keyholder
  capabilities: string[];         // ["echo", "web_search", "generate_image"]
  network: {
    protocol:  'http' | 'websocket' | 'grpc' | 'tcp' | 'libp2p';
    host:      string;            // "192.168.1.5" or "agent.example.com"
    port:      number;
    tls:       boolean;
    peerId?:   string;            // libp2p PeerId (when protocol === 'libp2p')
    multiaddr?: string;           // full multiaddr, e.g. "/ip4/…/udp/…/quic-v1/p2p/…"
  };
  health: {
    status:         'healthy' | 'degraded' | 'unhealthy';
    lastHeartbeat:  string;       // ISO 8601
    uptimeSeconds:  number;
  };
  registeredAt: string;           // ISO 8601
  metadataUri?: string;           // IPFS / HTTPS link to extended metadata
}
```

The DHT stores a signed envelope around this record:

```json
{
  "v": 1,
  "seq": 42,
  "entry": { /* StoredEntry fields */ },
  "sig": "<base64-compact-secp256k1-signature-over-sha256(unsigned-envelope)>"
}
```

### Capabilities

Capabilities are **plain strings** declared by `getCapabilities()`. They form the unit of discovery and billing:

```typescript
getCapabilities(): string[]   // e.g. ["echo", "web_search", "generate_image"]
```

For plugin-wrapped agents a `CapabilityDescriptor` carries richer metadata:

```typescript
interface CapabilityDescriptor {
  name:          string;
  description:   string;
  inputSchema?:  Record<string, unknown>;   // JSON Schema
  outputSchema?: Record<string, unknown>;   // JSON Schema
  pricePerCall?: string;                    // "0.05 USDC" — triggers x402 gate
}
```

When `pricePerCall` is set the HTTP server automatically returns HTTP 402 on calls that carry no payment proof.

---

### `POST /invoke` — AgentRequest / AgentResponse

**Request**

```typescript
interface AgentRequest {
  requestId:  string;                        // UUID v4
  from:       string;                        // caller agentId or wallet address
  capability: string;                        // target capability name
  payload:    Record<string, unknown>;       // capability-specific body
  signature?: string;                        // EIP-712 signature over the envelope
  timestamp?: number;                        // Unix ms — used to reject stale calls
  sessionKey?: string;                       // delegated execution session
  payment?:   { type, token, amount, txHash? };  // legacy payment field
  x402?:      X402Payment;                   // x402 micropayment proof (auto-attached by X402Client)
  stream?:    boolean;                       // true → use POST /invoke/stream (SSE)
}
```

**Response**

```typescript
interface AgentResponse {
  requestId:  string;
  status:     'success' | 'error' | 'payment_required';
  result?:    Record<string, unknown>;       // present on success
  errorMessage?: string;                     // present on error / payment_required
  proof?:     string;                        // optional ZK proof or attestation
  signature?: string;                        // EIP-712 response signature
  timestamp?: number;                        // Unix ms
  paymentRequirements?: X402PaymentRequirements[];  // present on payment_required
}
```

**Wire example**

```json
// POST /invoke
{ "requestId": "a1b2", "from": "sentrix://agent/caller", "capability": "web_search",
  "payload": { "query": "latest AI news" }, "timestamp": 1711234567000 }

// 200 OK
{ "requestId": "a1b2", "status": "success",
  "result": { "content": "…", "sources": ["…"] }, "timestamp": 1711234567120 }

// 402 Payment Required
{ "requestId": "a1b2", "status": "payment_required",
  "errorMessage": "Capability 'generate_image' requires payment.",
  "paymentRequirements": [{ "network": "base", "asset": "0x833…", "maxAmountRequired": "50000", "payTo": "0xYour…" }] }
```

---

### Mesh Protocols — Heartbeat, Capability Exchange, Gossip

These are dispatched via the same `POST /invoke` endpoint using **reserved capability names**.

#### Heartbeat — `__heartbeat`

```typescript
// AgentRequest.capability = "__heartbeat"
// AgentRequest.payload cast to:
interface HeartbeatRequest {
  senderId:  string;
  timestamp: number;   // Unix ms
  nonce?:    string;
}

// AgentResponse.result cast to:
interface HeartbeatResponse {
  agentId:           string;
  status:            'healthy' | 'degraded' | 'unhealthy';
  timestamp:         number;
  capabilitiesCount: number;
  uptimeMs?:         number;
  version?:          string;
  nonce?:            string;   // echoed from request
}
```

#### Capability Exchange — `__capabilities`

```typescript
// AgentRequest.payload cast to:
interface CapabilityExchangeRequest {
  senderId:   string;
  timestamp:  number;
  includeAnr: boolean;   // true → response includes full DiscoveryEntry
}

// AgentResponse.result cast to:
interface CapabilityExchangeResponse {
  agentId:      string;
  capabilities: string[];
  timestamp:    number;
  anr?:         DiscoveryEntry;   // present when includeAnr was true
}
```

#### Gossip — `POST /gossip`

Gossip is **fire-and-forget** — the server always returns `{ "ok": true }`. Messages propagate hop by hop; each hop decrements `ttl` and appends its own ID to `seenBy` to prevent loops.

```typescript
interface GossipMessage {
  type:        'announce' | 'revoke' | 'heartbeat' | 'query';
  senderId:    string;
  timestamp:   number;           // Unix ms
  ttl:         number;           // decremented each hop; dropped at 0
  seenBy:      string[];         // agent IDs that have already forwarded this
  entry?:      DiscoveryEntry;   // present for announce / revoke
  capability?: string;           // present for query
  nonce?:      string;
}
```

**Wire example**

```json
// POST /gossip
{ "type": "announce", "senderId": "sentrix://agent/0xABC",
  "timestamp": 1711234567000, "ttl": 3, "seenBy": [],
  "entry": { "agentId": "sentrix://agent/0xABC", "capabilities": ["web_search"], … } }

// 200 OK
{ "ok": true }
```

---

### Streaming — `POST /invoke/stream`

Set `AgentRequest.stream = true` (or call `POST /invoke/stream` directly). The server responds with `Content-Type: text/event-stream` and emits SSE frames until the terminal `StreamEnd` frame.

```typescript
// Each incremental frame:
interface StreamChunk {
  requestId: string;
  type:      'chunk';
  delta:     string;       // LLM token text or incremental output
  result?:   unknown;      // optional partial structured result
  sequence:  number;       // monotonically increasing per request
  timestamp: number;
}

// Terminal frame:
interface StreamEnd {
  requestId:    string;
  type:         'end';
  finalResult?: unknown;   // fully assembled result
  error?:       string;    // set on abnormal termination
  sequence:     number;
  timestamp:    number;
}
```

**Wire example**

```
data: {"type":"chunk","requestId":"a1b2","delta":"The ","sequence":1,"timestamp":1711234567100}

data: {"type":"chunk","requestId":"a1b2","delta":"latest ","sequence":2,"timestamp":1711234567110}

data: {"type":"end","requestId":"a1b2","finalResult":{"text":"The latest AI news…"},"sequence":47,"timestamp":1711234567890}
```

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

config = PluginConfig(**identity.to_plugin_config_fields(), port=6174)
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
- [ ] Rust + Zig gossip discovery implementation
- [ ] Streaming responses (SSE / WebSocket) for Rust and Zig
- [ ] Public hosted discovery registry
- [ ] ERC-8004 delegation (`checkPermission`) on-chain enforcement

---

## License

Apache 2.0 — see [LICENSE](LICENSE)
