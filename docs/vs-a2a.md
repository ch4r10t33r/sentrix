# Borgkit vs A2A — What Problem Does Each Solve?

> **Short answer:** A2A defines *how* two agents talk. Borgkit defines *how agents find each other, prove who they are, establish trust, and transact* — problems A2A explicitly leaves out of scope.

They are more complementary than competing. A2A is a communication standard; Borgkit is coordination infrastructure. But if you are building an open, permissionless agent mesh without existing enterprise infrastructure, Borgkit solves problems A2A deliberately does not.

---

## What A2A Actually Is

A2A (Agent-to-Agent protocol, now under the Linux Foundation) standardises multi-turn task exchange between agents over HTTP/JSON-RPC or gRPC. It is well-designed and pragmatic:

- **Agent Cards** — agents publish a JSON metadata document at `/.well-known/agent-card.json` describing their skills, capabilities, and required authentication schemes.
- **Task lifecycle** — a well-defined state machine: `submitted → working → input_required | completed | failed | canceled`.
- **Transport** — HTTP(S) with JSON-RPC 2.0 or gRPC; SSE for streaming; webhooks for async push notifications.
- **Auth** — delegates entirely to existing standards: OAuth 2.0, API keys, mTLS, OIDC. No new auth primitives.
- **Content** — flexible `Part` model: text, binary files, structured JSON.

A2A deliberately keeps its scope narrow. The specification explicitly marks the following as **out of scope**:

| Out of scope in A2A | Reference |
|---|---|
| How agents discover each other (no registry defined) | A2A spec §Discovery |
| Attestation that an Agent Card matches a real agent | A2A spec §Security |
| Authorization enforcement (left to each agent) | A2A spec §Security |
| Credential provisioning (out-of-band, each org decides) | A2A spec §Auth |
| Delegation auditing — tracking who delegated to whom | A2A spec §Limitations |
| Sensitive data handling, GDPR/HIPAA controls | A2A spec §Limitations |
| End-to-end encryption at the message level | Not defined |
| Payment primitives | Not defined |
| Agents behind NAT / no public HTTP endpoint | Not defined |

These are not criticisms — A2A made conscious trade-offs to stay simple and reuse existing standards. But they are real gaps that production deployments must fill somehow.

---

## What Borgkit Adds

Borgkit fills exactly those gaps. Here is a precise breakdown:

### 1. Permissionless Discovery

**A2A:** You must know an agent's domain to fetch its Agent Card (`/.well-known/agent-card.json`). For a curated registry, someone must administer it. Discovery is not defined by the protocol.

**Borgkit:** Every agent announces itself to a Kademlia DHT (capability key = `SHA-256("borgkit:cap:<capability>")`). Any node can find any agent by capability with no prior knowledge of its URL, domain, or IP. mDNS handles LAN discovery. No admin, no registry, no URL exchange required.

```bash
# An agent deployed anywhere on the internet is immediately findable:
borgkit discover --capability translate
# → returns matching agents from the DHT, including their multiaddrs and DID
```

### 2. Cryptographic Agent Identity

**A2A:** Agent Cards are self-declared. No mechanism verifies that the card matches the actual agent. Identity is URL-based — change the domain, lose your identity.

**Borgkit:** Every agent has a `did:key` derived from a secp256k1 or X25519 keypair. The DID is:
- **Portable** — survives IP/domain changes
- **Verifiable** — any party can verify a signature against the public key embedded in the DID
- **Self-sovereign** — no certificate authority, no DNS dependency

The Agent Network Record (ANR) — Borgkit's equivalent of an Agent Card — is signed by the agent's DID key. Recipients can verify it.

### 3. Authenticated + Encrypted Messaging

**A2A:** TLS protects the transport (server-to-client). There is no end-to-end encryption at the message level. If the server is compromised, or messages pass through an intermediary, content is exposed.

**Borgkit:** DIDComm v2 provides end-to-end encrypted, authenticated messages between agents:
- **Authcrypt** — recipient knows and can verify the sender's identity
- **Anoncrypt** — anonymous sender, recipient cannot identify origin
- Crypto: X25519 ECDH key agreement + ChaCha20-Poly1305 AEAD — zero infrastructure required beyond the keypair

```typescript
// Alice sends an encrypted, authenticated task to Bob
// Bob can verify it came from Alice — no OAuth server, no CA
const encrypted = await alice.invoke(bob.did, 'translate', { text: 'hello' });
const { message, senderDid } = await bob.unpack(encrypted);
```

### 4. Payments at the Protocol Layer

**A2A:** No payment primitive. Billing, rate limiting, and monetisation are out of scope.

**Borgkit:** Two complementary payment paths:

- **x402** — micropayments built in. Agents can charge per invocation, accept payment before running a task, and issue receipts verifiable on-chain (USDC / ETH on Base, etc.).
- **MPP (Machine Payments Protocol)** — explicit first-class support via the **MPP plugin** in Borgkit templates ([mpp.dev](https://mpp.dev)): HTTP **402** payment-required responses, challenge–credential–receipt flow, with **Tempo** stablecoin, **Stripe** Secure Payment Tokens (SPT), or **Lightning** depending on configuration. Ship-ready in **TypeScript**, **Rust**, and **Zig** scaffolds; Python template support is planned.

This enables an open market of agent services — anyone can deploy an agent and charge for it, without integrating a separate billing system, and agents can interoperate with wallets and paymasters that speak MPP.

### 5. True Peer-to-Peer — No Public Endpoint Required

**A2A:** Strictly client-server. The agent must have a publicly addressable HTTPS endpoint. Agents behind NAT cannot participate without a reverse proxy or tunnel.

**Borgkit:** Agents connect to the libp2p mesh via QUIC. Circuit relay allows agents behind NAT or firewalls to be reachable. The same mesh protocol works on a LAN (mDNS) and across the open internet (DHT + relay).

This matters for:
- **Edge/IoT agents** — a Zig agent running on embedded hardware with no public IP
- **On-premise agents** — enterprise agents that cannot expose an HTTP endpoint to the public internet
- **Consumer devices** — agents running on a laptop or mobile device

### 6. Capability-Based Routing, Not URL-Based

**A2A:** You invoke a specific agent at a specific URL. You need prior knowledge of which agent provides what.

**Borgkit:** You query by capability. The mesh returns the best available agents that provide it, with health scores and latency. The client picks one (or fans out to several). This enables:
- **Load balancing** across multiple agents with the same capability
- **Failover** — if one agent goes down, route to another automatically
- **Capability negotiation** — find the agent that supports both `translate` and `summarise`

### 7. Cross-Framework, Cross-Language Mesh

**A2A:** Framework-agnostic in theory, but most implementations are Python or TypeScript. There is no native concept of "this agent runs LangGraph" vs "this agent runs CrewAI".

**Borgkit:** First-class framework plugins for LangGraph, Google ADK, CrewAI, OpenAI Agents SDK, Agno, LlamaIndex, smolagents, and MCP — in TypeScript, Rust, and Zig. A LangGraph agent and a CrewAI agent and a Rust agent all speak the same Borgkit mesh protocol. Framework identity is part of the ANR.

### 8. Embedded and Resource-Constrained Deployment

**A2A:** Built around HTTP servers. The minimum viable implementation needs an HTTP server capable of handling JSON-RPC 2.0.

**Borgkit:** Zig templates implement the full mesh protocol (discovery, invocation, gossip, DIDComm) using only `std.net` and `std.crypto` — no HTTP framework, no runtime dependencies. Agents can run on embedded hardware with kilobytes of RAM.

---

## Feature Comparison

| Feature | A2A | Borgkit |
|---|---|---|
| **Multi-turn task protocol** | ✅ Well-defined state machine | ✅ `/invoke` + `/invoke/stream` |
| **Agent metadata / skills** | ✅ Agent Card | ✅ Agent Network Record (ANR) |
| **Streaming** | ✅ SSE | ✅ SSE |
| **Transport** | HTTP/JSON-RPC, gRPC | HTTP + libp2p QUIC |
| **Authentication** | OAuth2, API keys, mTLS (delegated) | DID signatures, DIDComm v2 |
| **End-to-end encryption** | ❌ Transport-only (TLS) | ✅ DIDComm v2 (X25519 + ChaCha20) |
| **Cryptographic agent identity** | ❌ Self-declared, unverified | ✅ `did:key` — keypair-derived, verifiable |
| **Permissionless discovery** | ❌ Requires known URL or managed registry | ✅ Kademlia DHT + mDNS gossip |
| **Capability-based routing** | ❌ You call a specific URL | ✅ Query by capability, mesh returns candidates |
| **Payments** | ❌ Out of scope | ✅ x402 micropayments · **MPP** (HTTP 402, Tempo / Stripe / Lightning) in TS, Rust, Zig templates |
| **Agents behind NAT** | ❌ Requires public HTTP endpoint | ✅ Circuit relay via libp2p |
| **Offline resilience** | ❌ Request fails if server down | ✅ DHT caches records, mesh reroutes |
| **Identity attestation** | ❌ Explicitly out of scope | ✅ Signed ANR records |
| **Delegation auditing** | ❌ Explicitly out of scope | ✅ DID-traceable invocation chain |
| **Embedded / Zig / no-runtime** | ❌ Requires HTTP server | ✅ Pure Zig, std-only |
| **Framework plugins** | ❌ Agnostic (bring your own) | ✅ LangGraph, CrewAI, OpenAI, ADK, Agno, … |
| **Enterprise OAuth / OIDC** | ✅ First-class | 🔜 Possible via HTTP layer |
| **Task state machine** | ✅ Rich (8 states, artifacts, pagination) | Simpler (`/invoke` response) |
| **Linux Foundation backing** | ✅ | Community-driven |
| **Multi-modal content (files, data)** | ✅ Parts model | Via `/invoke` payload |

---

## When to Use Each

### Use A2A when:

- You are integrating with existing enterprise infrastructure (OAuth2, OIDC, existing agent registries)
- You need a rich, standardised task lifecycle with artifacts, multi-turn conversations, and webhooks
- Your agents all have publicly accessible HTTPS endpoints
- Interoperability with the broader A2A ecosystem (Google ADK, LangGraph, etc.) is a priority
- Simplicity of implementation matters — A2A is easier to implement from scratch

### Use Borgkit when:

- You need agents to find each other without a central registry or prior URL exchange
- You need verifiable agent identity that survives IP/domain changes
- You need end-to-end encrypted messages between agents (not just TLS)
- You are building a monetised agent marketplace (x402 and/or **MPP** for HTTP 402–based machine payments)
- Your agents run behind NAT, on edge devices, or in embedded environments
- You want one protocol that works LAN-local (mDNS) and globally (DHT) without configuration

### Use both together:

Borgkit handles **discovery and identity**; A2A handles **task communication**. A Borgkit agent can expose an A2A-compatible `/invoke` endpoint — letting it be discovered via Borgkit's DHT and invoked using the A2A task protocol. The two layers are orthogonal.

```
Borgkit DHT          → finds the agent's multiaddr + DID
DIDComm v2           → authenticates and encrypts the request
A2A task protocol    → structures the multi-turn conversation
x402 / MPP           → handles payment for the task (on-chain micropayments or MPP challenge–credential–receipt)
```

---

## The One-Paragraph Summary

A2A is a well-designed protocol for *how* two agents communicate — it defines the message envelope, task lifecycle, and authentication delegation. It deliberately does not define how agents find each other, how they prove who they are cryptographically, how they handle end-to-end encryption, or how they get paid. Borgkit fills exactly those gaps: a Kademlia DHT for permissionless capability-based discovery, `did:key` for portable cryptographic identity, DIDComm v2 for end-to-end encrypted authenticated messaging, x402 for built-in micropayments, and **MPP** ([Machine Payments Protocol](https://mpp.dev)) for standards-aligned HTTP 402 agent payments (Tempo, Stripe SPT, Lightning) in TypeScript, Rust, and Zig templates. If A2A is the postal standard, Borgkit is the address book, the envelope seal, and the stamp.
