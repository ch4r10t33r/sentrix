# Borgkit — Overview

Borgkit is a **decentralised agent networking framework** that lets you build, discover, and interconnect software agents without relying on any centralised service.

Think of it as **HTTP + DNS for agents** — but peer-to-peer, cryptographically verifiable, and framework-agnostic.

---

## Core ideas

### 1. Agents are first-class network citizens

Every agent in Borgkit has a verifiable identity (backed by a secp256k1 key pair), a list of capabilities it exposes, and a signed network record — the **ANR** — that encodes all of this into a compact, portable string.

```
anr:enqFiWFtcC12MWlkYHNlcHAy...
```

That one string is everything another agent needs to find and call you.

### 2. No central registry required

Discovery is **pluggable** and defaults to fully local/P2P operation:

| Mode | When to use |
|---|---|
| `LocalDiscovery` | Development & unit tests (in-process) |
| `HttpDiscovery` | Managed staging / enterprise environments |
| `GossipDiscovery` *(coming)* | Production P2P mesh |
| `OnChainDiscovery` *(coming)* | ERC-8004 Ethereum-native registry |

Switch between them with a single environment variable (`BORGKIT_DISCOVERY_URL`) or a one-line config change — **your agent code never changes**.

### 3. Framework-agnostic by design

Borgkit does not care how your agent is built. Bring your existing LangGraph graph, Google ADK agent, or custom logic and wrap it with a **BorgkitPlugin** in one function call:

```python
agent = wrap_langgraph(graph, name="Researcher", agent_id="borgkit://agent/researcher")
await agent.register_discovery()
```

### 4. ERC-8004 identity model

Agent identities are grounded in the **ERC-8004** standard:

- Every agent has an `agentId` URI (`borgkit://agent/<address>`)
- Identity is backed by a secp256k1 key pair — the same cryptographic primitive used by Ethereum
- Ownership, delegation, and permissions follow the ERC-8004 model
- On-chain anchoring is optional but natively supported

---

## Repository layout

```
borgkit/
├── src/                        # CLI source (TypeScript)
│   ├── cli.ts                  # Entry point & command wiring
│   ├── version.ts              # Single source of version truth
│   └── commands/               # init · create · run · discover · version
│       └── utils/              # logger · generator · detect-lang
│
├── templates/                  # Per-language project scaffolds
│   ├── typescript/             # TS interfaces, agents, discovery, ANR, plugins
│   ├── python/                 # Python equivalents
│   ├── rust/                   # Rust equivalents
│   └── zig/                    # Zig equivalents
│
└── docs/                       # You are here
    ├── overview.md             # This file
    ├── anr.md                  # Agent Network Record spec
    ├── interfaces.md           # IAgent · IAgentRequest · IAgentResponse · IAgentDiscovery
    ├── discovery.md            # Discovery layer & adapters
    ├── plugins.md              # BorgkitPlugin · LangGraph · Google ADK
    ├── version-management.md   # Versioning & changelog policy
    └── examples/
        ├── 01-hello-agent.md         # Defining your first agent
        ├── 02-agent-discovery.md     # Making an agent discoverable
        ├── 03-agent-to-agent.md      # Calling another agent
        ├── 04-multi-agent-workflow.md # Chaining agents
        └── 05-cross-framework.md     # LangGraph ↔ Google ADK interop
```

---

## Quick start

```bash
npm install -g borgkit-cli

borgkit init my-project --lang python
cd my-project
pip install -r requirements.txt
python -m agents.example_agent
```

For all options: `borgkit --help`

---

## Protocol spec modules

| Module | Status | Description |
|---|---|---|
| **AMP-1** | ✅ stable | Discovery — ANR encoding, query protocol |
| **AMP-2** | ✅ stable | Interaction — `AgentRequest` / `AgentResponse` wire format |
| **AMP-3** | 🚧 draft | Payments — stream, oneshot, subscription |
| **AMP-4** | 🚧 draft | Delegation & multi-agent workflows |
