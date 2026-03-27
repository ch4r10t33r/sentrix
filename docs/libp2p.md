# libp2p Integration

Borgkit uses **libp2p** as its P2P discovery and transport layer — the same
networking stack that powers Ethereum, Filecoin, and Polkadot.

> **Future path:** [iroh](https://iroh.computer) (QUIC-native, dial-by-NodeId)
> is tracked as a potential replacement once it reaches 1.0 and matures its
> Python/Zig support.  The `IAgentDiscovery` interface is the abstraction
> boundary — a future `IrohDiscovery` would be a drop-in swap.

---

## Why libp2p

| Requirement | How libp2p meets it |
|---|---|
| Capability-keyed discovery | Kademlia DHT provider records — `provide(cap_cid)` / `findProviders(cap_cid)` |
| NAT traversal | DCUtR hole punching + circuit relay v2 fallback |
| QUIC transport | `rust-libp2p quic` feature (quinn), `@chainsafe/libp2p-quic` in TypeScript |
| ANR identity | secp256k1 PeerId — same keypair as ANR, one identity everywhere |
| Local LAN | mDNS (zero config, built in) |
| Cross-language | Rust + TypeScript primary; Python via sidecar |
| Production proof | Ethereum Beacon Chain, Filecoin (~3 200 nodes), 210k IPFS nodes |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Borgkit Application                          │
│  IAgent.register()  →  IAgentDiscovery.register(DiscoveryEntry)    │
└──────────────────────────────┬──────────────────────────────────────┘
                               │
                    ┌──────────▼──────────┐
                    │   Libp2pDiscovery   │  implements IAgentDiscovery
                    │  (TS / Rust / Py)   │
                    └──────────┬──────────┘
                               │
          ┌────────────────────┼─────────────────────┐
          │                    │                      │
   ┌──────▼──────┐    ┌────────▼───────┐    ┌────────▼──────┐
   │  Kademlia   │    │     mDNS       │    │  Circuit      │
   │  DHT        │    │  (LAN only)    │    │  Relay v2     │
   │  /borgkit/  │    │                │    │  (NAT fallback│
   │  kad/1.0.0  │    └────────────────┘    └───────────────┘
   └──────┬──────┘
          │
   ┌──────▼──────┐    ┌──────────────┐
   │  QUIC       │◄───│  DCUtR       │
   │  transport  │    │  hole punch  │
   │  (UDP only) │    └──────────────┘
   └─────────────┘
```

The Borgkit DHT is **isolated** from the public IPFS DHT via the custom
protocol string `/borgkit/kad/1.0.0`.  Borgkit agents only peer with other
Borgkit agents; they do not participate in IPFS routing.

---

## DHT Key Schema

All DHT keys are namespaced with `borgkit:` to avoid collisions.

| Record type | Key | Value | Purpose |
|---|---|---|---|
| Provider record | `SHA256("borgkit:cap:<capability>")` → CIDv1 | PeerId list (managed by libp2p) | Capability advertisement |
| Value record | `SHA256("borgkit:anr:<agentId>")` | Signed JSON envelope | Full DiscoveryEntry |
| Reverse map | `/borgkit/pid/<peerId>` | UTF-8 agentId | PeerId → agentId lookup |

### Capability CID (cross-language)

The same input string is used in all languages, ensuring TypeScript and Rust
peers find each other's provider records:

```
input  = UTF-8("borgkit:cap:" + capability)
hash   = SHA2-256(input)
cid    = CIDv1(codec=0x55/raw, hash)
```

### DHT value envelope

The full `DiscoveryEntry` is wrapped in a signed envelope before being stored:

```json
{
  "v":     1,
  "seq":   42,
  "entry": { ...DiscoveryEntry... },
  "sig":   "<base64url 64-byte compact secp256k1 signature>"
}
```

Signed bytes: `keccak256(UTF-8("borgkit:anr:v1:") + JSON.stringify(entry))`

Consumers **must** verify the signature and reject envelopes with `seq` lower
than a previously seen value for the same `agentId`.

---

## Identity: One Keypair, One Identity

```
ANR secp256k1 private key (32 bytes)
         │
         ▼
libp2p Secp256k1Keypair
         │
         ├──► PeerId  (used for libp2p routing, dialing, identify)
         │
         └──► ANR record signature  (used to authenticate DiscoveryEntry)
```

The ANR public key IS the libp2p peer identity.  A single 32-byte private key
controls both layers.

---

## Heartbeat in P2P Context

There is no central server to ping.  Instead, heartbeat means
**re-publishing DHT records before they expire**:

| HTTP Discovery | libp2p P2P equivalent |
|---|---|
| `PUT /agents/:id/hb` to registry | Re-publish value record + provider records |
| Server marks entry healthy | Updated `lastHeartbeat` in re-published envelope |
| Server evicts stale entries | DHT records expire; staleness applied at read time |

**Staleness heuristic** (applied by `query` and `listAll` at read time):

| Age of `lastHeartbeat` | Status applied |
|---|---|
| < 5 minutes | `healthy` |
| 5 – 15 minutes | `degraded` |
| > 15 minutes | `unhealthy` (excluded from `query` results) |

Default heartbeat interval: **30 seconds**.

---

## NAT Traversal

```
Scenario 1 — Both peers reachable (no NAT):
  Agent A  ──[QUIC direct]──►  Agent B

Scenario 2 — One peer behind NAT:
  Agent A  ──[QUIC hole punch via DCUtR]──►  Agent B
  (coordinated through a relay peer in the routing table)

Scenario 3 — Both peers behind strict NAT / symmetric firewall:
  Agent A  ──[QUIC via circuit relay]──►  Relay  ──[QUIC]──►  Agent B
  (relay server must be reachable by both; Borgkit bootstrap peers can serve as relays)
```

Note: Circuit relay connections are TCP under the hood (libp2p relay v2 uses
TCP + Noise + Yamux).  The Borgkit peer itself never opens a TCP *listener*,
but it may open outbound TCP connections to relay servers.

---

## Bootstrap Peers

Bootstrap peers seed the Kademlia routing table on startup.  Resolution order:

1. `bootstrapPeers` / `bootstrap_peers` in config
2. `BORGKIT_BOOTSTRAP_PEERS` env var (comma-separated multiaddrs)
3. Built-in Borgkit public bootstrap nodes (added when deployed)
4. mDNS peers on the local network (automatic, no config needed)

Multiaddr format for QUIC:
```
/ip4/1.2.3.4/udp/4001/quic-v1/p2p/12D3KooW...
```

Always use `/quic-v1` (not the older `/quic` draft variant).

### Peer cache

After 10 minutes of uptime, the node persists newly discovered stable peers to
`~/.borgkit/peer-cache.json`.  On restart, these cached peers are tried before
the fallback bootstrap list, improving resilience if bootstrap nodes change.

---

## Configuration Reference

### TypeScript

```typescript
import { Libp2pDiscovery } from './discovery/Libp2pDiscovery';

const discovery = await Libp2pDiscovery.create({
  privateKey:         myAnrPrivateKey,      // 32-byte Uint8Array
  listenAddresses:    ['/ip4/0.0.0.0/udp/0/quic-v1'],
  bootstrapPeers:     ['/ip4/1.2.3.4/udp/4001/quic-v1/p2p/12D3KooW...'],
  heartbeatIntervalMs: 30_000,
  enableMdns:          true,
  dhtClientMode:       false,
});
```

Or via `DiscoveryFactory`:

```typescript
const discovery = await DiscoveryFactory.create({
  type:   'libp2p',
  libp2p: { privateKey: myAnrPrivateKey },
});
```

Or via environment:
```bash
export BORGKIT_P2P=true
export BORGKIT_BOOTSTRAP_PEERS=/ip4/1.2.3.4/udp/4001/quic-v1/p2p/12D3KooW...
```

### Python

```python
from discovery.libp2p_discovery import Libp2pDiscovery, Libp2pDiscoveryConfig

cfg = Libp2pDiscoveryConfig(
    private_key_bytes     = my_anr_key,
    listen_port           = 4001,
    bootstrap_peers       = ['/ip4/1.2.3.4/udp/4001/quic-v1/p2p/12D3KooW...'],
    heartbeat_interval_secs = 30,
    enable_mdns           = True,
)
discovery = await Libp2pDiscovery.start(cfg)
```

Python requires the `borgkit-libp2p-sidecar` binary or Node.js in PATH.
See the [sidecar section](#python-sidecar) below.

### Rust

```rust
use borgkit::discovery_libp2p::{Libp2pDiscovery, Libp2pDiscoveryConfig};

let cfg = Libp2pDiscoveryConfig {
    private_key_bytes: my_anr_key,
    listen_port:       4001,
    bootstrap_peers:   vec![],
    heartbeat_secs:    30,
    enable_mdns:       true,
    dht_client_mode:   false,
};
let discovery = Libp2pDiscovery::start(cfg).await?;
```

---

## Python Sidecar

py-libp2p is incomplete, so Python agents use a lightweight Rust/TypeScript
sidecar process.  The sidecar speaks JSON-RPC 2.0 over stdin/stdout:

```
Python ──[stdin]──► sidecar ──[libp2p QUIC DHT]──► mesh
Python ◄─[stdout]── sidecar
```

**Build the Rust sidecar:**
```bash
cd templates/rust
cargo build --release --bin borgkit-libp2p-sidecar
export BORGKIT_LIBP2P_SIDECAR=$(pwd)/target/release/borgkit-libp2p-sidecar
```

**Use the TypeScript sidecar (requires Node >= 20):**
```bash
export BORGKIT_LIBP2P_NODE=templates/typescript/discovery/libp2p-sidecar.js
```

**HTTP gateway fallback** (if neither is available):
```bash
export BORGKIT_LIBP2P_GATEWAY=http://localhost:7731
```
The gateway is an HTTP server that bridges to the libp2p DHT.

---

## `listAll` Behaviour

`listAll()` is a best-effort operation in P2P mode.  Unlike HTTP discovery,
there is no server-side list of all agents.  The implementation returns:

1. All **locally registered** agents (authoritative, zero latency).
2. Remote agents discovered via DHT walks or accumulated `query` calls.

Do not rely on `listAll()` for exhaustive enumeration.  Use it for monitoring
and debugging only.  For production orchestration, use `query(capability)` which
is O(log N) in the DHT.

---

## Comparison with Other Backends

| | LocalDiscovery | HttpDiscovery | Libp2pDiscovery |
|---|---|---|---|
| **Scope** | Single process | Single registry server | Global P2P mesh |
| **Setup** | Zero config | Needs a server | Bootstrap peers |
| **Latency** | Microseconds | ~10 ms | 100 ms – 1 s (first query) |
| **NAT traversal** | N/A | Server-mediated | DCUtR + relay |
| **Fault tolerance** | Process restart | Server SPOF | No SPOF |
| **Use case** | Dev / test | Enterprise / hosted | Production P2P |

---

## Future: iroh

[iroh](https://iroh.computer) is tracked as a future replacement for the
transport layer.  Key differences from libp2p:

- **QUIC Multipath** — libp2p does not yet have this; iroh fully supports it
- **Relay as QUIC path** — iroh's relay is a first-class QUIC path migration,
  not a separate circuit; relay→direct upgrade happens in-flight
- **Dial by NodeId** — iroh requires no IP address at all; the NodeId IS the
  address (maps perfectly to ANR identity)
- **Simpler stack** — iroh replaces both libp2p transport AND the relay
  abstraction with a single QUIC-based system

When iroh reaches 1.0 and Python/Zig support matures, `IrohDiscovery` will be
added as a new backend.  The `IAgentDiscovery` interface ensures zero code
changes for callers.
