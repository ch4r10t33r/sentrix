# SentriX: Differentiation & Positioning

## Overview

SentriX is a protocol and framework designed to enable **agent-to-agent (A2A) discovery and communication across heterogeneous systems**.

While existing frameworks focus on building and orchestrating agents, SentriX focuses on enabling agents to **find, verify, and interact with each other across boundaries**.

---

## The Current Landscape

Most existing agent systems fall into three categories:

### 1. Orchestration Frameworks
Examples: CrewAI, AutoGen

- Agents operate within a controlled environment
- Typically follow manager → worker patterns
- Strong at task coordination

**Limitations:**
- No open discovery mechanism
- Agents cannot interact outside their runtime
- No interoperability across frameworks

---

### 2. Framework-Bound Agents
Examples: LangGraph, AutoGPT

- Agents are tightly coupled to a specific framework
- Communication happens within predefined graphs or workflows

**Limitations:**
- No cross-framework compatibility
- No standardized interface for external interaction
- Limited extensibility beyond the framework

---

### 3. Networked but Closed Ecosystems
Examples: Fetch.ai, SingularityNET

- Provide identity, discovery, and communication
- Enable agent marketplaces and interactions

**Limitations:**
- Ecosystem lock-in
- Heavyweight infrastructure
- Not optimized for developer-first workflows

---

## The Missing Layer

There is currently no system that provides:

- A **framework-agnostic agent interface**
- A **standard discovery protocol**
- A **lightweight communication layer**
- **Interoperability across runtimes** (e.g., LangGraph ↔ Google ADK)

---

## SentriX Approach

SentriX introduces a **protocol layer for agents**, analogous to how TCP/IP enables communication across heterogeneous systems.

### Core Principles

1. **Framework Agnostic**
   - Agents can be built using any framework (LangGraph, Google ADK, etc.)
   - SentriX provides adapters/plugins for interoperability

2. **Discoverability by Design**
   - Agents can register capabilities
   - Other agents can query and discover them dynamically

3. **Standardized Interface**
   - Common request/response schema
   - Capability-based interaction model

4. **Decoupled Communication**
   - Agents communicate over a protocol, not via shared memory or orchestration
   - Enables true peer-to-peer interaction

5. **Composable Agents**
   - Agents can call other agents
   - Enables chaining, delegation, and emergent workflows

---

## Architectural Positioning

| Layer | Responsibility |
|------|--------------|
| L1 | Identity, capability, trust — DID (W3C) by default; ERC-8004 on-chain optional |
| L2 | Discovery (registry, indexing, P2P lookup) |
| L3 | Interaction (request/response protocol) |
| L4 | Execution (agent runtime, frameworks like LangGraph) |

SentriX primarily operates in **L2 and L3**, while integrating with L1 and enabling L4.

---

## Key Differentiators

### 1. Interoperability First
SentriX enables agents built on different frameworks to communicate seamlessly.

### 2. Open Discovery Protocol
Agents are not pre-wired; they are dynamically discoverable.

### 3. Protocol, Not Platform
SentriX is not a closed ecosystem. It is a **standard layer** that others can build on.

### 4. Lightweight & Developer-Friendly
Designed for rapid agent creation (`borgkit init`) and minimal boilerplate.

### 5. Extensible by Design
Supports:
- Plugins for different frameworks
- Custom discovery backends
- Optional trust, payment, and verification layers

---

## Positioning Statement

> "Existing frameworks help you build agents.  
> SentriX helps agents find and talk to each other."

---

## Vision

SentriX aims to become the **default coordination layer for autonomous agents**, enabling:

- Open agent ecosystems
- Cross-platform collaboration
- Decentralized AI networks
- W3C DID-native agent identity (no wallet required)
- Composable, discoverable intelligence

---

## Analogy

If current frameworks are like **applications**,
SentriX is the **internet protocol layer** that connects them.

---

## Borgkit vs. A2A (Agent2Agent Protocol)

### What A2A is

- **JSON-RPC 2.0 over HTTP(S)** — agents call each other via fixed URLs, like microservices
- **Agent Card** — a static JSON document at `/.well-known/agent.json` advertising capabilities
- **Task model** — formal long-running task lifecycle (submitted → working → completed/failed)
- **SSE streaming + push notifications** — for async responses
- **Multi-language SDKs** — Python, Go, JS, Java, .NET
- **Backed by Google + Linux Foundation** — enterprise credibility, formal governance

### Side-by-side

| Dimension | A2A | Borgkit |
|---|---|---|
| **Transport** | HTTP(S) JSON-RPC — fixed URLs | libp2p P2P mesh — no fixed addresses |
| **Discovery** | Agent Card at known URL (pull) | Gossip-based — agents announce, peers propagate |
| **Payments** | Not in scope | x402 native, per-capability pricing; **MPP** ([Machine Payments Protocol](https://mpp.dev)) plugins in TypeScript, Rust, and Zig (HTTP 402, Tempo / Stripe SPT / Lightning) |
| **Framework wrapping** | Implement A2A protocol directly in your agent | Plugin system — wrap any existing agent without rewriting |
| **MCP interop** | No | Bidirectional MCP bridge |
| **Task lifecycle** | Formal (submitted → working → complete) | Request/response + streaming, no formal task state machine |
| **CLI scaffolding** | No | `borgkit init`, `borgkit create agent`, `borgkit run` |
| **Language SDKs** | Python, Go, JS, Java, .NET | Python, TypeScript (Rust CLI in progress) |
| **Governance** | Google / Linux Foundation | Independent |

### Where Borgkit is structurally different

**1. P2P vs. client-server topology**

A2A assumes agents have stable HTTP URLs. Agent A calling Agent B requires knowing B's URL ahead of
time — fundamentally a microservices model for agents.

Borgkit assumes agents may be ephemeral, mobile, or behind NAT. libp2p handles peer addressing, hole
punching, and routing. An agent can join and leave the mesh without a static URL.

**2. Discovery model**

A2A discovery is pull-based at a known address — you have to know where to look (`.well-known/`
endpoint convention).

Borgkit uses gossip propagation — when an agent registers, its ANR (Agent Network Record) spreads to
all peers automatically. Any agent can ask "who can do X?" without knowing URLs in advance.

**3. Framework plugin architecture**

A2A requires implementing their protocol in your agent. Borgkit uses a plugin adaptor pattern —
`wrap_langchain()`, `wrap_openai()`, `wrap_mcp()` — your existing agent becomes Borgkit-native in
one line, with capabilities extracted automatically from the framework's own metadata.

**4. Payments as a first-class primitive**

A2A has deferred payments to a future roadmap. Borgkit has **x402** gating for on-chain micropayments and, separately, **MPP (Machine Payments Protocol)** support via plugins in the TypeScript, Rust, and Zig templates — HTTP 402 challenge–credential–receipt flows with Tempo stablecoin, Stripe Secure Payment Tokens, or Lightning ([mpp.dev](https://mpp.dev)). Capabilities become billable services without inventing a custom payment protocol.

