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

## LocalDiscovery (default)

**Backend:** in-process `HashMap` / `dict`
**Use when:** development, unit tests, single-process multi-agent setups

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
2. BORGKIT_DISCOVERY_URL env var  →  HttpDiscovery
3. (default)                      →  LocalDiscovery
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
Is this a local dev / test scenario?
  → LocalDiscovery  (default, no config needed)

Do you have a managed registry or are running in an enterprise network?
  → HttpDiscovery   (set BORGKIT_DISCOVERY_URL)

Are you running a production P2P mesh with untrusted peers?
  → GossipDiscovery (coming)

Do you need cryptoeconomic trust and on-chain verifiability?
  → OnChainDiscovery (coming)
```

All four modes use the identical `IAgentDiscovery` interface — switching never requires changing agent logic.
