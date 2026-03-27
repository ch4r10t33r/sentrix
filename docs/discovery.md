# Discovery Layer

Borgkit's discovery layer is **pluggable** — you swap backends without changing a single line of agent code. This document describes each adapter, when to use it, and how the `DiscoveryFactory` selects between them.

---

## How discovery works

When an agent starts up it calls `register_discovery()`. This pushes a `DiscoveryEntry` to whatever backend is active. When another agent wants to find a peer with a certain capability, it calls `query(capability)` and gets back a list of live entries.

```
AgentA.register_discovery()  →  registry.register(entry)
AgentB.query("getWeather")   →  registry.query("getWeather")  →  [AgentA's entry]
AgentB calls AgentA at entry.network.host:port
```

Heartbeats keep entries alive. If an agent stops sending heartbeats, the registry marks it `unhealthy` and excludes it from query results.

---

## LocalDiscovery

**Backend:** in-process `HashMap` / `dict`
**Use when:** unit tests, single-process multi-agent demos, CI environments where opening ports is undesirable
**Enable with:** `BORGKIT_DISCOVERY_TYPE=local`

```typescript
// TypeScript — singleton, shared across all agents in the process
const registry = LocalDiscovery.getInstance();
await registry.register(entry);
const results  = await registry.query('getWeather');
```

```python
# Python — same singleton pattern
registry = LocalDiscovery.get_instance()
await registry.register(entry)
results  = await registry.query('getWeather')
```

No network calls. Zero latency. Perfect for running multiple agents in one process during development.

---

## HttpDiscovery (optional, centralised)

**Backend:** any HTTP server that implements the Borgkit registry REST API
**Use when:** managed staging, enterprise environments, bootstrapping a new P2P network

### Activating HttpDiscovery

Three ways to enable it:

**1. Environment variable (no code change)**
```bash
export BORGKIT_DISCOVERY_URL=https://registry.example.com
export BORGKIT_DISCOVERY_KEY=my-api-key   # optional
python -m agents.my_agent
```

**2. DiscoveryFactory config**
```typescript
const registry = DiscoveryFactory.create({
  type: 'http',
  http: { baseUrl: 'https://registry.example.com', apiKey: 'my-key' }
});
```

**3. Direct instantiation**
```python
from discovery.http_discovery import HttpDiscovery
registry = HttpDiscovery(base_url='https://registry.example.com', api_key='my-key')
```

### Registry REST API contract

Any server can act as an HttpDiscovery registry as long as it exposes:

| Method | Path | Description |
|---|---|---|
| `POST` | `/agents` | Register an agent (body: `DiscoveryEntry` JSON) |
| `DELETE` | `/agents/:id` | Unregister an agent |
| `GET` | `/agents?cap=X` | Query agents by capability |
| `GET` | `/agents` | List all agents |
| `PUT` | `/agents/:id/hb` | Heartbeat (keep-alive) |

### Automatic heartbeats

`HttpDiscovery` starts a background task that sends a heartbeat every 30 seconds (configurable). If the agent process exits unexpectedly, the registry will eventually mark it unhealthy via TTL.

```python
HttpDiscovery(
    base_url             = 'https://registry.example.com',
    heartbeat_interval_ms = 15_000,   # every 15s
    timeout_ms           = 3_000,
)
```

---

## DiscoveryFactory — auto-selection

`DiscoveryFactory` applies the following priority order:

```
1. Explicit type in config / constructor argument
2. BORGKIT_DISCOVERY_TYPE env var  →  local | http | libp2p
3. BORGKIT_DISCOVERY_URL env var   →  HttpDiscovery
4. (default)                       →  Libp2pDiscovery
                                      (falls back to LocalDiscovery if libp2p fails to bind)
```

```typescript
// TypeScript
import { DiscoveryFactory } from './discovery/DiscoveryFactory';

const registry = DiscoveryFactory.create();
// → LocalDiscovery in dev, HttpDiscovery in prod (via env var)
```

```python
# Python
from discovery.http_discovery import DiscoveryFactory

registry = DiscoveryFactory.create()
```

```rust
// Rust
let registry = DiscoveryFactory::from_env();
// → AnyDiscovery::Local or AnyDiscovery::Http
```

---

## GossipDiscovery (coming — AMP-1 Phase 2)

P2P gossip-based discovery with no central server.

**How it will work:**
- Agents maintain a partial view of the network (k random peers)
- On startup, connect to any known bootstrap peer
- Periodically exchange known agents with random peers
- Entries propagate across the whole mesh within seconds
- Dead agents are eventually garbage-collected via TTL + heartbeat

Target: ~10,000 agents, <5s propagation time, zero central dependency.

---

## OnChainDiscovery (coming — AMP-1 Phase 3)

ERC-8004 on-chain registry adapter.

**How it will work:**
- Agents publish their ANR to the ERC-8004 registry contract
- Discovery queries index the chain via a Subgraph or direct RPC
- Identity and capability claims become **cryptoeconomically verifiable**
- Revocation happens on-chain — no stale entries survive chain re-orgs

---

## Choosing a discovery mode

```
Default (no env vars set)?
  → Libp2pDiscovery  (Kademlia DHT, mDNS on LAN, graceful fallback to local on bind failure)

Do you need in-process discovery only (unit tests, single-process demos)?
  → LocalDiscovery   (BORGKIT_DISCOVERY_TYPE=local — no ports opened)

Do you have a managed registry or are running in an enterprise network?
  → HttpDiscovery    (BORGKIT_DISCOVERY_TYPE=http + BORGKIT_DISCOVERY_URL=…)

Do you need cryptoeconomic trust and on-chain verifiability?
  → OnChainDiscovery (coming)
```

All four modes use the identical `IAgentDiscovery` interface — switching never requires changing agent logic.

### Environment variable quick reference

| Env var | Purpose | Example |
|---|---|---|
| `BORGKIT_DISCOVERY_TYPE` | Force a specific mode | `local` \| `http` \| `libp2p` |
| `BORGKIT_DISCOVERY_URL` | HTTP registry URL (activates `http` if `TYPE` unset) | `https://registry.example.com` |
| `BORGKIT_DISCOVERY_KEY` | API key for HTTP registry | `sk-…` |
| `BORGKIT_AGENT_KEY` | 64-hex secp256k1 key for stable Peer ID | `openssl rand -hex 32` |
| `BORGKIT_BOOTSTRAP_PEERS` | Comma-separated multiaddrs for libp2p | `/ip4/1.2.3.4/tcp/6174/p2p/12D3…` |

---

## Troubleshooting

### "There is no Peer ID in the banner"

A Peer ID only exists when a libp2p host is running. In `local` and `http` modes there is no libp2p host, so no Peer ID is generated. This is expected behaviour.

To enable libp2p (and get a real Peer ID), set in your `.env`:

```env
BORGKIT_DISCOVERY_TYPE=libp2p
BORGKIT_AGENT_KEY=<your 64-hex secp256k1 private key>
```

If `BORGKIT_AGENT_KEY` is omitted, a random ephemeral key is generated each startup — the Peer ID will differ between runs.

### "The agent says discovery type is local but I set BORGKIT_DISCOVERY_URL"

`BORGKIT_DISCOVERY_URL` activates `http` discovery, not `libp2p`. To activate libp2p you need `BORGKIT_DISCOVERY_TYPE=libp2p` (the URL is unused in libp2p mode). The DiscoveryFactory priority is:

```
1. Explicit type  →  BORGKIT_DISCOVERY_TYPE=http|libp2p
2. URL present    →  BORGKIT_DISCOVERY_URL set  →  http discovery
3. Default        →  local
```

### "Agents can't find each other across machines"

- **`local` mode** — in-process only; agents on different machines cannot see each other. Switch to `http` or `libp2p`.
- **`http` mode** — both agents must point to the same `BORGKIT_DISCOVERY_URL`.
- **`libp2p` mode** — agents need at least one shared bootstrap peer. Set `BORGKIT_BOOTSTRAP_PEERS=/ip4/<peer-ip>/tcp/6174/p2p/<PeerId>` or use mDNS on the same LAN (automatic, no config needed).
