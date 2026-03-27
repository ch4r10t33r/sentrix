<p align="center">
  <img src="https://raw.githubusercontent.com/ch4r10t33r/borgkit/main/docs/logo.svg" alt="Borgkit" width="220"/>
</p>

# Borgkit

> **Autonomous Agentic Coordination Middleware** — scaffold P2P-discoverable, DID-native AI agents that interoperate across any framework, with optional ERC-8004 on-chain compliance.

Like TCP/IP connects heterogeneous computers, Borgkit connects heterogeneous agents.

---

## Why Borgkit?

Most AI frameworks help you **build** agents. Borgkit helps them **find and talk to each other** — across frameworks, runtimes, and clouds.

| What exists today | The gap Borgkit fills |
|---|---|
| Orchestration frameworks (CrewAI, AutoGen) | Agents locked inside one runtime, no external discovery |
| Framework-bound agents (LangGraph, AutoGPT) | No standardised interface for cross-framework calls |
| Closed ecosystems (Fetch.ai, SingularityNET) | Ecosystem lock-in, heavyweight infrastructure |

Borgkit is **not a platform** — it is a protocol layer others build on.

---

## Architecture

| Layer | Role | Technologies |
|---|---|---|
| **L4** Execution | Agent frameworks | LangGraph · Google ADK · CrewAI · Agno · LlamaIndex · smolagents · OpenAI Agents |
| **L3** Interaction | Request / response | `AgentRequest` / `AgentResponse` · AMP-2 |
| **L2** Discovery | Capability lookup | Local · HTTP · libp2p + Kademlia DHT · AMP-1 |
| **L1** Identity | DID + trust | `did:key` W3C (default) · ERC-8004 on-chain (optional) |

Borgkit operates primarily at **L2** and **L3**, bridging L1 identity to L4 framework execution.

---

## Features

- **Framework-agnostic** — wrap LangGraph, Google ADK, CrewAI, Agno, LlamaIndex, smolagents, or OpenAI Agents with one function call
- **Built-in HTTP server** — `borgkit run MyAgent --port 6174` starts a real HTTP server; no extra setup
- **MCP bridge** — any MCP server becomes a Borgkit agent; any Borgkit agent becomes an MCP server (Claude Desktop, Cursor, Continue)
- **Dynamic discovery** — agents register capabilities; callers query at runtime, no hardcoded URLs
- **Mesh protocols** — heartbeat, capability exchange (as part of handshake), and gossip fan-out built in
- **P2P mesh** — libp2p + QUIC + Kademlia DHT; mDNS for LAN; circuit relay for NAT traversal
- **DID identity** — `did:key` W3C standard out of the box; no wallet, no gas, no tokens required
- **x402 payments** — opt-in micropayment layer; charge per capability in USDC / ETH on Base
- **MPP payments** — [Machine Payments Protocol](https://mpp.dev) plugin; HTTP 402 challenge–credential–receipt flow with Tempo stablecoin, Stripe SPT, and Lightning support across TypeScript, Rust, and Zig
- **Multi-language** — TypeScript, Python, Rust, Zig
- **One CLI** — scaffold, create, run, discover

---

## Language Coverage

| Feature | Python | TypeScript | Rust | Zig |
|---|:---:|:---:|:---:|:---:|
| **IAgent interface** | ✅ | ✅ | ✅ | ✅ |
| **AgentRequest / AgentResponse** | ✅ | ✅ | ✅ | ✅ |
| **ANR (Agent Network Record)** | ✅ | ✅ | ✅ | ✅ |
| **DID identity (`did:key`)** | ✅ | ✅ | ✅ | ✅ |
| **HTTP server (`borgkit run`)** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — local (in-memory)** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — HTTP** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — libp2p + Kademlia DHT** | ✅ | ✅ | ✅ | ✅ |
| **Discovery — gossip fan-out** | ✅ | ✅ | ✅ | ✅ |
| **AgentClient (mesh protocols)** | ✅ | ✅ | ✅ | ✅ |
| **Example agent** | ✅ | ✅ | ✅ | ✅ |
| **Plugin system (framework adapters)** | ✅ | ✅ | ✅ | ✅ |
| **LangGraph plugin** | ✅ | ✅ | ✅ | ✅ |
| **Google ADK plugin** | ✅ | ✅ | ✅ | ✅ |
| **CrewAI plugin** | ✅ | ✅ | ✅ | ✅ |
| **OpenAI Agents SDK plugin** | ✅ | ✅ | ✅ | ✅ |
| **Agno plugin** | ✅ | ✅ | ✅ | ✅ |
| **LlamaIndex plugin** | ✅ | ✅ | ✅ | ✅ |
| **smolagents plugin** | ✅ | ✅ | ✅ | ✅ |
| **MCP bridge (wrap MCP servers)** | ✅ | ✅ | ✅ | ✅ |
| **MCP bridge (expose as MCP server)** | ✅ | ✅ | ✅ | ✅ |
| **x402 micropayments** | ✅ | ✅ | ✅ | ✅ |
| **MPP plugin ([Machine Payments Protocol](https://mpp.dev))** | 🔜 | ✅ | ✅ | ✅ |
| **Streaming (SSE via /invoke/stream)** | ✅ | ✅ | ✅ | ✅ |

**Legend:** ✅ implemented · 🔜 on roadmap · — not applicable for this language

**MPP:** HTTP 402 payment gating using the MPP challenge–credential–receipt model (`MppPlugin` / `mpp` modules). Python template support is on the roadmap; use x402 or bridge via a TS/Rust/Zig agent until then.

**Rust / Zig — framework plugins:** LangGraph, Google ADK, and CrewAI are Python/JS frameworks with no native Rust or Zig SDKs. The Rust and Zig plugins are **HTTP bridge adapters** that call a running service endpoint (e.g. `adk web`, a LangServe app, or a FastAPI-wrapped CrewAI crew) so the agent participates in the Borgkit mesh without embedding a Python interpreter.

**Zig — discovery:** `HttpDiscovery` (`discovery_http.zig`) is a REST client for the discovery service. `Libp2pDiscovery` (`discovery_libp2p.zig`) is a full Kademlia DHT implementation in pure Zig (UDP/JSON transport, 256-bucket XOR routing table, k=20). Key derivation matches the Rust implementation (`SHA-256("borgkit:cap:<cap>")` / `SHA-256("borgkit:anr:<agentId>")`). Note: uses a Borgkit-native JSON wire format rather than libp2p protobuf — interoperates with other Zig Borgkit nodes.

---

## Installation

### npm (recommended)

Works on macOS, Linux, and Windows. npm downloads the correct pre-built
binary for your platform automatically.

```bash
npm install -g @ch4r10teer41/borgkit-cli
```

### curl installer (macOS / Linux)

Auto-detects your OS and architecture, installs to `/usr/local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/borgkit/main/install.sh | sh
```

To install to a custom directory set `BORGKIT_INSTALL_DIR` before piping:

```bash
BORGKIT_INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/borgkit/main/install.sh | sh
```

> **Windows** — use npm above, or download `borgkit-win32-x64.exe` directly from the
> [Releases page](https://github.com/ch4r10t33r/borgkit/releases/latest).

### Build from source

Requires [Rust](https://rustup.rs) 1.75+.

```bash
git clone https://github.com/ch4r10t33r/borgkit.git
cd borgkit
cargo build --release --package borgkit-cli
# Binary is at ./target/release/borgkit
```

---

## Quick Start

```bash
# Scaffold a new project (TypeScript default)
borgkit init my-agent
cd my-agent && npm install
borgkit run ExampleAgent --port 6174

# Python
borgkit init my-agent --lang python
cd my-agent
borgkit run ExampleAgent --port 6174

# Rust
borgkit init my-agent --lang rust
cd my-agent
cargo run --example did_key_identity        # optional: did:key from secp256k1 secret
cargo run --example gossip_fanout_discovery # optional: in-memory gossip fan-out demo

# Zig
borgkit init my-agent --lang zig
cd my-agent
zig build examples   # optional: builds did:key + gossip fan-out demo binaries
```

Once running, the agent prints its full startup banner:

```
────────────────────────────────────────────────────────────
  Borgkit Agent Online  v0.1.0
────────────────────────────────────────────────────────────
  Name         ExampleAgent
  Agent ID     borgkit://agent/example
  Endpoint     http://0.0.0.0:6174
  Multiaddr    /ip4/0.0.0.0/tcp/6174/p2p/12D3Koo...  (libp2p mode)
  Discovery    local
  Capabilities (2)
           • echo
           • ping
────────────────────────────────────────────────────────────
```

> **Default port: 6174** ([Kaprekar's constant](https://en.wikipedia.org/wiki/6174)). Override with `BORGKIT_PORT=<n>` or `--port <n>`.

---

## CLI Reference

| Command | Description |
|---|---|
| `borgkit scaffold <name> [OPTIONS]` | Generate a minimal, targeted agent project (see below) |
| `borgkit init <name> [--lang ts\|python\|rust\|zig]` | Copy the full template library into a new project (see below) |
| `borgkit create agent <name> [-c cap1,cap2] [--framework X]` | Add an agent to an existing project |
| `borgkit run <AgentName> [--port 6174]` | Start an agent's HTTP server |
| `borgkit discover [-c capability] [--host h] [--port p]` | Query the discovery layer |
| `borgkit version` | Show CLI version and build info |

### `borgkit scaffold` vs `borgkit init`

Both create a new agent project, but they serve different workflows:

| | `scaffold` | `init` |
|---|---|---|
| **Approach** | Generates files programmatically from flags | Copies the full embedded template library |
| **Output** | Minimal — only what you asked for | Full kitchen sink — all discovery adapters, example agents, every template file |
| **Customisation** | `--plugins`, `--stream`, `--x402`, `--did`, `--discovery` flags wire things together for you | Raw templates with `{{AGENT_NAME}}` token substitution — you wire things yourself |
| **Also generates** | `.env.example`, `README.md` | `borgkit.config.json` |
| **Best for** | Starting a focused, production-ready agent quickly | Exploring the full template library or building something custom |
| **Languages** | TypeScript, Rust, Zig | TypeScript, Python, Rust, Zig |

> **Rule of thumb:** use `scaffold` when you know what you want; use `init` when you want to browse all available patterns and pick your own path.

### `borgkit scaffold` — targeted project generator

```bash
borgkit scaffold <name> [OPTIONS]

Options:
  -l, --lang <LANG>           typescript | rust | zig          [default: typescript]
  -p, --plugins <PLUGINS>     openai,agno,langgraph,google_adk,crewai,
                              llamaindex,smolagents,mcp        [default: none]
  -o, --output <DIR>          output directory                 [default: cwd]
  -d, --did                   include DID key generation example
  -s, --stream                include SSE streaming endpoint
  -x, --x402                  include x402 micropayments middleware
      --discovery <BACKEND>   http | libp2p                    [default: http]
      --dry-run               print file tree without writing
```

**Examples:**

```bash
# TypeScript agent with LangGraph + OpenAI, SSE streaming, libp2p discovery
borgkit scaffold my-agent --lang typescript --plugins langgraph,openai --stream --discovery libp2p

# Rust agent with MCP bridge and x402 micropayments
borgkit scaffold payments-agent --lang rust --plugins mcp --x402

# Zig agent with DID examples — preview first, then generate
borgkit scaffold did-agent --lang zig --did --dry-run
borgkit scaffold did-agent --lang zig --did
```

Generated structure (TypeScript example):
```
my-agent/
├── package.json
├── tsconfig.json
├── src/
│   ├── agent.ts        ← discovery registration, /invoke handler, selected plugins
│   ├── index.ts        ← entry point
│   └── plugins/        ← only created when --plugins is set
│       └── LangGraphPlugin.ts
├── .env.example
└── README.md
```

### `borgkit init` — full template copy

```bash
borgkit init <name> [OPTIONS]

Options:
  -l, --lang <LANG>     typescript | python | rust | zig   [default: typescript]
      --no-discovery    skip copying discovery adapter files
      --no-example      skip copying example agent files
```

Copies the **entire template library** for the chosen language into `<name>/`, applies token substitution (`{{AGENT_NAME}}`, `{{PROJECT_NAME}}`, etc.), and writes a `borgkit.config.json`. The result is a fully-populated project containing every discovery adapter, all plugin stubs, and complete example agents — ready to explore and trim down.

```bash
# Full TypeScript project with everything included
borgkit init my-project --lang typescript

# Rust project without the example agent files
borgkit init my-project --lang rust --no-example
```

### `borgkit run` — HTTP endpoints

When you run an agent, these endpoints are live automatically:

| Endpoint | Method | Description |
|---|---|---|
| `/invoke` | `POST` | Call any capability — `{ capability, payload, from }` → `AgentResponse` |
| `/invoke/stream` | `POST` | Same as `/invoke` but responds with `text/event-stream` SSE frames |
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
agent = wrap_google_adk(adk_agent, name="SupportBot", agent_id="borgkit://agent/support", owner="0x...")

# CrewAI
from plugins.crewai_plugin import wrap_crewai
agent = wrap_crewai(crew_agent, name="ResearchBot", agent_id="borgkit://agent/research", owner="0x...")

# LangGraph
from plugins.langgraph_plugin import wrap_langgraph
agent = wrap_langgraph(compiled_graph, config)

# OpenAI Agents SDK
from plugins.openai_plugin import wrap_openai
agent = wrap_openai(oai_agent, name="WeatherBot", agent_id="borgkit://agent/weather", owner="0x...")

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
// LangGraph (in-process — wraps a compiled CompiledGraph)
import { wrapLangGraph } from './plugins/LangGraphPlugin';
const agent = wrapLangGraph(compiledGraph, { agentId: 'borgkit://agent/researcher', name: 'Researcher', ... });

// Google ADK (in-process — wraps a BaseAgent / LlmAgent)
import { wrapGoogleADK } from './plugins/GoogleADKPlugin';
const agent = wrapGoogleADK(adkAgent, { agentId: 'borgkit://agent/support', name: 'Support', ... });

// CrewAI (HTTP bridge — calls a running CrewAI service)
import { wrapCrewAI } from './plugins/CrewAIPlugin';
const agent = await wrapCrewAI({
  agentId:    'borgkit://agent/writer',
  name:       'WriterCrew',
  version:    '1.0.0',
  owner:      '0xYourWallet',
  serviceUrl: 'http://localhost:8000',  // FastAPI-wrapped CrewAI crew
});

// OpenAI Agents SDK
import { wrapOpenAI }     from './plugins/OpenAIPlugin';
import { AgnoPlugin }     from './plugins/AgnoPlugin';
import { LlamaIndexPlugin } from './plugins/LlamaIndexPlugin';
import { SmolagentsPlugin } from './plugins/SmolagentsPlugin';

const agent = wrapOpenAI(oaiAgent, { agentId: 'borgkit://agent/weather', name: 'WeatherBot', ... });

// Agno, LlamaIndex, smolagents — same one-liner pattern
const agnoAgent     = new AgnoPlugin({ agentId: 'borgkit://agent/agno', ... }).wrap(myAgnoAgent);
const llamaAgent    = new LlamaIndexPlugin({ agentId: 'borgkit://agent/llama', ... }).wrap(myIndex);
const smolaAgent    = new SmolagentsPlugin({ agentId: 'borgkit://agent/smol', ... }).wrap(mySmolAgent);

await agent.serve({ port: 6174 });
```

### Rust

```rust
use borgkit::plugins::{
    langgraph::{LangGraphPlugin, LangGraphService},
    google_adk::{GoogleADKPlugin, GoogleADKService},
    crewai::{CrewAIPlugin, CrewAIService},
    openai::{OpenAIPlugin, OpenAIService},
    agno::{AgnoPlugin, AgnoService},
    llamaindex::{LlamaIndexPlugin, LlamaIndexService},
    smolagents::{SmolagentsPlugin, SmolagentsService},
    base::PluginConfig,
};

// LangGraph — HTTP bridge to a LangServe endpoint
let service = LangGraphService { base_url: "http://localhost:8000".into(), ..Default::default() };
let agent = LangGraphPlugin::new().wrap(service, PluginConfig {
    agent_id: "borgkit://agent/researcher".into(), owner: "0xYourWallet".into(), ..Default::default()
});

// OpenAI-compatible API (OpenAI, vLLM, Ollama, …)
let service = OpenAIService {
    base_url: "https://api.openai.com".into(),
    model:    "gpt-4o-mini".into(),
    api_key:  Some(std::env::var("OPENAI_API_KEY").unwrap()),
    ..Default::default()
};
let agent = OpenAIPlugin::new().wrap(service, PluginConfig { .. });

// Agno — HTTP bridge to a deployed Agno FastAPI server
let service = AgnoService { base_url: "http://localhost:7777".into(), ..Default::default() };
let agent = AgnoPlugin::new().wrap(service, PluginConfig { .. });

// LlamaIndex — HTTP bridge to a LlamaIndex server
let service = LlamaIndexService { base_url: "http://localhost:8080".into(), ..Default::default() };
let agent = LlamaIndexPlugin::new().wrap(service, PluginConfig { .. });

// smolagents — Gradio or custom API bridge
let service = SmolagentsService { base_url: "http://localhost:7860".into(), ..Default::default() };
let agent = SmolagentsPlugin::new().wrap(service, PluginConfig { .. });

// CrewAI — HTTP bridge to a FastAPI-wrapped crew
let mut service = CrewAIService { base_url: "http://localhost:8000".into(), ..Default::default() };
let plugin = CrewAIPlugin::new();
plugin.fetch_capabilities(&mut service).await?;
let agent = plugin.wrap(service, PluginConfig { .. });
```

### Zig

```zig
const lg   = @import("plugins/langgraph.zig");
const adk  = @import("plugins/google_adk.zig");
const ca   = @import("plugins/crewai.zig");
const oai  = @import("plugins/openai.zig");
const agno = @import("plugins/agno.zig");
const lli  = @import("plugins/llamaindex.zig");
const sma  = @import("plugins/smolagents.zig");
const Wrapped = @import("plugins/wrapped_agent.zig").WrappedAgent;

// LangGraph — HTTP bridge to a LangServe endpoint
var lg_service = lg.LangGraphService{ .base_url = "http://localhost:8000" };
var lg_plugin  = lg.LangGraphPlugin.init(allocator);
defer lg_plugin.deinit();
var agent = Wrapped(lg.LangGraphService, lg.LangGraphPlugin).init(
    &lg_service, &lg_plugin, .{ .agent_id = "borgkit://agent/researcher", .owner = "0x..." }, allocator,
);

// OpenAI-compatible API
var oai_service = oai.OpenAIService{ .base_url = "https://api.openai.com", .api_key = "sk-..." };
var oai_plugin  = oai.OpenAIPlugin.init(allocator);
defer oai_plugin.deinit();

// Agno — deployed Agno server
var agno_service = agno.AgnoService{ .base_url = "http://localhost:7777" };
var agno_plugin  = agno.AgnoPlugin.init(allocator);
defer agno_plugin.deinit();

// LlamaIndex — deployed LlamaIndex server
var lli_service = lli.LlamaIndexService{ .base_url = "http://localhost:8080" };

// smolagents — Gradio or custom API
var sma_service = sma.SmolagentsService{ .base_url = "http://localhost:7860" };
```

---

## MCP Bridge

Borgkit has a two-way bridge with the [Model Context Protocol](https://modelcontextprotocol.io):

```python
# Any MCP server → Borgkit agent (GitHub, filesystem, Slack, databases…)
from plugins.mcp_plugin import MCPPlugin
plugin = await MCPPlugin.from_command(
    ["npx", "-y", "@modelcontextprotocol/server-github"],
    config, env={"GITHUB_TOKEN": "ghp_..."}
)
agent = plugin.wrap()
await agent.serve(port=8081)

# Any Borgkit agent → MCP server (Claude Desktop, Cursor, Continue…)
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

```rust
// Rust — stdio subprocess or HTTP endpoint
use borgkit::mcp::{McpPlugin, serve_as_mcp, ServeMcpOptions, Transport};
use borgkit::plugins::base::PluginConfig;

// Wrap an MCP server (subprocess) → Borgkit agent
let agent = McpPlugin::from_command(
    &["npx", "-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
    PluginConfig { agent_id: "borgkit://agent/fs".into(), .. },
    None,
).await?;
borgkit::server::serve(agent, 8081).await?;

// Expose a Borgkit agent → MCP server (stdio for Claude Desktop)
serve_as_mcp(my_agent, ServeMcpOptions::default()).await?;

// Expose over SSE (remote clients)
serve_as_mcp(my_agent, ServeMcpOptions {
    transport: Transport::Sse, port: 3000, ..Default::default()
}).await?;
```

```zig
// Zig — stdio subprocess or HTTP endpoint
const mcp_plugin = @import("mcp_plugin.zig");
const mcp_server = @import("mcp_server.zig");

// Wrap an MCP server (subprocess) → Borgkit agent
var plugin = mcp_plugin.McpPlugin.initStdio(allocator);
defer plugin.deinit();
try plugin.fromCommand(&.{ "npx", "-y", "@modelcontextprotocol/server-filesystem", "/tmp" }, null);

// Expose a Borgkit agent → MCP server (stdio for Claude Desktop)
try mcp_server.serveAsMcp(MyAgent, &my_agent, .{}, allocator);

// Expose over HTTP (POST /mcp)
try mcp_server.serveAsMcp(MyAgent, &my_agent, .{ .transport = .http, .port = 3000 }, allocator);
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

## DIDComm v2 — Encrypted Agent Messaging

Borgkit includes a full DIDComm v2 implementation for end-to-end encrypted, authenticated messages between agents. No external key infrastructure required — each agent's `did:key` is derived from its identity keypair.

**Crypto stack:** X25519 ECDH key agreement + ChaCha20-Poly1305 AEAD.
**Wire format:** JWE JSON serialization with per-recipient key wrapping.
**Modes:** `authcrypt` (sender-authenticated) and `anoncrypt` (anonymous).

### TypeScript

```typescript
import { DidcommClient, MessageTypes } from './didcomm';

const alice = DidcommClient.generateKeyPair();
const bob   = DidcommClient.generateKeyPair();

const aliceClient = new DidcommClient(alice);
const bobClient   = new DidcommClient(bob);

// Alice encrypts an INVOKE message to Bob (authcrypt)
const encrypted = await aliceClient.invoke(bob.did, 'translate', { text: 'hello' });

// Bob decrypts it
const { message, senderDid } = await bobClient.unpack(encrypted);
console.log(message.body);   // { text: 'hello' }
console.log(senderDid);      // alice's did:key
```

### Rust

```rust
use crate::didcomm::DidcommClient;

let alice = DidcommClient::generate()?;
let bob   = DidcommClient::generate()?;

// Encrypt (authcrypt)
let packed = alice.invoke(&bob.did, "translate", json!({"text": "hello"}), false)?;

// Decrypt
let (msg, sender_did) = bob.unpack(&packed)?;
println!("{}", msg.body);           // {"text":"hello"}
println!("{:?}", sender_did);       // Some("did:key:z6Mk...")

// Anonymous — sender is not revealed
let anon = alice.invoke(&bob.did, "translate", json!({"text": "hi"}), true)?;
let (msg2, none_sender) = bob.unpack(&anon)?;
assert!(none_sender.is_none());
```

### Zig

```zig
const didcomm = @import("didcomm.zig");

var alice = try didcomm.DidcommClient.generate(allocator);
defer alice.deinit();
var bob = try didcomm.DidcommClient.generate(allocator);
defer bob.deinit();

// Encrypt
const packed = try alice.invoke(allocator, bob.did, "translate", "{\"text\":\"hello\"}", false);
defer allocator.free(packed);

// Decrypt
const result = try bob.unpack(allocator, packed);
defer result.deinit(allocator);
std.debug.print("body: {s}\n", .{result.message.body_json});
std.debug.print("from: {?s}\n", .{result.sender_did});
```

Source: [`templates/typescript/didcomm.ts`](templates/typescript/didcomm.ts) · [`templates/rust/src/didcomm.rs`](templates/rust/src/didcomm.rs) · [`templates/zig/src/didcomm.zig`](templates/zig/src/didcomm.zig)

---

## Protocol Reference

### Ports and Addresses

| Layer | Default address | Env override |
|---|---|---|
| HTTP server (`/invoke`, `/health`, …) | `0.0.0.0:6174` | `BORGKIT_PORT` |
| libp2p TCP (GossipSub + request-response) | `/ip4/0.0.0.0/tcp/6174` | `BORGKIT_P2P_ADDR` |
| libp2p QUIC (Rust/Zig DHT) | `/ip4/0.0.0.0/udp/6174/quic-v1` | `BORGKIT_P2P_PORT` |
| Bootstrap peers | _(none — mDNS only on LAN)_ | `BORGKIT_BOOTSTRAP_PEERS` (comma-separated multiaddrs) |

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
| `local` (default) | `did:key:zQ3sh…` | `borgkit://agent/<eth-addr>` |
| `env` | `did:key:zQ3sh…` | same, driven by `BORGKIT_AGENT_KEY` |
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
  agentId:      string;           // "borgkit://agent/0xABC…" or a DID
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
{ "requestId": "a1b2", "from": "borgkit://agent/caller", "capability": "web_search",
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
{ "type": "announce", "senderId": "borgkit://agent/0xABC",
  "timestamp": 1711234567000, "ttl": 3, "seenBy": [],
  "entry": { "agentId": "borgkit://agent/0xABC", "capabilities": ["web_search"], … } }

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
| `local` (default) | `did:key:z...` | Key auto-created in `~/.borgkit/keystore/` |
| `env` | `did:key:z...` | `BORGKIT_AGENT_KEY=0x...` env var |
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
git clone https://github.com/ch4r10t33r/borgkit
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
| [docs/plugins.md](docs/plugins.md) | Framework adapters — LangGraph, Google ADK, CrewAI, OpenAI, Agno, LlamaIndex, smolagents, MCP |
| [docs/differentiation.md](docs/differentiation.md) | How Borgkit differs from other frameworks |
| [docs/vs-a2a.md](docs/vs-a2a.md) | Borgkit vs A2A — detailed technical comparison |

---

## Borgkit vs A2A

> **A2A defines *how* two agents talk. Borgkit defines *how agents find each other, prove who they are, and transact* — problems A2A explicitly leaves out of scope.**

| | A2A | Borgkit |
|---|---|---|
| Discovery | You need the agent's URL; bring your own registry | Kademlia DHT — find any agent by capability, no URL needed |
| Identity | Self-declared Agent Card, no cryptographic verification | `did:key` — keypair-derived, portable, verifiable |
| Encryption | TLS (transport only) | DIDComm v2 — end-to-end, message-level |
| Payments | Out of scope | x402 micropayments built in |
| Network | Requires public HTTPS endpoint | P2P via libp2p QUIC; circuit relay for agents behind NAT |
| Routing | Call a specific URL | Query by capability; mesh returns candidates |
| Task model | Rich state machine (8 states, artifacts, webhooks) | Simple `/invoke` + `/invoke/stream` |
| Enterprise auth | OAuth2, OIDC, mTLS — first-class | Via HTTP layer |

They are **complementary, not competing.** A2A handles the task conversation; Borgkit handles discovery, identity, encryption, and payment. A Borgkit agent can expose an A2A-compatible endpoint — discovered via Borgkit's DHT and invoked using A2A's task protocol.

→ Full analysis: **[docs/vs-a2a.md](docs/vs-a2a.md)**

---

## TODOs

- [ ] More examples, tutorials and videos
- [ ] Public hosted discovery registry
- [ ] ERC-8004 delegation (`checkPermission`) on-chain enforcement
- [ ] True token-by-token streaming (requires agent-side `streamRequest` method)

---

## License

Apache 2.0 — see [LICENSE](LICENSE)
