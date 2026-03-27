# ANR — Agent Network Record

## What is ANR?

**ANR** stands for **Agent Network Record**.

It is the canonical, self-describing identity record for every agent in the Borgkit network. ANR is modelled directly on **EIP-778 (Ethereum Node Record, ENR)** — the same mechanism Ethereum nodes use to announce themselves on the devp2p network — and extends it with agent-specific fields.

Just as an ENR lets an Ethereum node say *"I am at this IP, on this port, with this public key"*, an ANR lets an agent say *"I am `borgkit://agent/0xABC`, I understand these capabilities, reachable here, owned by this wallet."*

---

## Why ANR?

Before ANR, agent identity was fragmented:
- Some frameworks use plain JSON config files
- Others rely on a central service handing out IDs
- None are **cryptographically verifiable** without external infrastructure

ANR solves this by packing identity, network reachability, and capability advertisement into a **single signed string** that:

1. **Self-authenticates** — the record includes a public key and a signature; anyone can verify it without phoning home
2. **Is portable** — copy-paste the `anr:…` string, paste it anywhere, the agent is reachable
3. **Is compact** — hard-capped at 512 bytes; fits in a UDP packet, a QR code, or an ENS text record
4. **Is extensible** — new key-value pairs can be added without breaking existing parsers
5. **Is Ethereum-native** — uses the same secp256k1 cryptography, making it trivially anchored on-chain via ERC-8004

---

## Anatomy of an ANR

### Text form

```
anr:enqFiWFtcC12MWlkZXNlbnRyaXg6Ly9hZ2VudC9leGFtcGxlYGEubmFtZWBXZWF0aGVyQWdlbnQ...
└──┘ └──────────────────────────────────────────────────────────────────────────────────┘
 prefix            base64url-encoded RLP payload (no padding)
```

The prefix `anr:` is analogous to `enr:` in Ethereum.

### Binary (wire) format

The binary payload is an **RLP-encoded list**:

```
[ signature, seq, k₁, v₁, k₂, v₂, … ]
```

| Field | Type | Description |
|---|---|---|
| `signature` | 64 bytes | secp256k1 compact signature (r‖s) over the content hash |
| `seq` | uint64 BE | Sequence number — incremented on every update |
| `k₁ … kₙ` | UTF-8 string | Key name (sorted lexicographically) |
| `v₁ … vₙ` | bytes | Key value (encoding depends on key) |

Keys are always **sorted lexicographically** and must be unique. This guarantees deterministic encoding and makes diffing two records trivial.

### What gets signed

The signature covers the **content RLP**, which is the same list *without* the signature, but with a domain separator prepended:

```
content = RLP( ["anr-v1", seq_be8, k₁, v₁, k₂, v₂, …] )
sig     = secp256k1_sign( keccak256(content), private_key )
```

The `"anr-v1"` domain separator prevents cross-protocol replay attacks — a valid ANR signature cannot be reused as an Ethereum transaction signature or vice versa.

---

## Key schema

Keys are divided into two namespaces:

### Network keys (inherited from EIP-778 ENR)

These are identical to ENR keys and ensure ANR records are parseable by ENR-aware tools.

| Key | Size | Description |
|---|---|---|
| `id` | string | Identity scheme — always `"amp-v1"` for Borgkit |
| `secp256k1` | 33 bytes | Compressed secp256k1 public key |
| `ip` | 4 bytes | IPv4 address (big-endian) |
| `ip6` | 16 bytes | IPv6 address |
| `tcp` | uint16 BE | TCP port |
| `udp` | uint16 BE | UDP port |

### Agent keys (Borgkit extensions, prefix `a.`)

All Borgkit-specific keys are prefixed with `a.` to avoid collision with future ENR keys.

| Key | Type | Description |
|---|---|---|
| `a.id` | UTF-8 string | Agent identifier URI, e.g. `borgkit://agent/0xABC` |
| `a.name` | UTF-8 string | Human-readable agent name |
| `a.ver` | UTF-8 string | Semantic version, e.g. `1.2.3` |
| `a.caps` | RLP list of strings | Capability names the agent exposes |
| `a.tags` | RLP list of strings | Searchable tags, e.g. `["weather","data"]` |
| `a.proto` | UTF-8 string | Transport hint: `http` \| `ws` \| `grpc` \| `tcp` |
| `a.port` | uint16 BE | Agent API port (may differ from `tcp`) |
| `a.tls` | 1 byte | `0x01` = TLS enabled, `0x00` = plaintext |
| `a.meta` | UTF-8 string | IPFS / Arweave URI for full metadata |
| `a.owner` | 20 bytes | EVM wallet or contract address of the owner |
| `a.chain` | uint64 BE | EVM chain ID (e.g. `1` = Ethereum mainnet) |

---

## Sequence numbers

`seq` is a **monotonically increasing uint64** that must be incremented every time the record is updated and republished. Recipients cache records by `(agentId, seq)` and always prefer the higher sequence number.

This is identical to ENR's behaviour: it provides authoritative updates without requiring a central coordinator.

---

## Size limit

An ANR record **must not exceed 512 bytes** in its binary RLP form. This is deliberately generous compared to ENR's 300-byte limit, to accommodate the additional agent-specific keys while still fitting in:
- A single UDP datagram
- An Ethereum transaction calldata field
- A DNS TXT record
- An ENS text record

---

## Identity scheme: `amp-v1`

The `id` key identifies the signing scheme. Borgkit defines one scheme: **`amp-v1`** (Agent Mesh Protocol v1).

`amp-v1` signing algorithm:

```
content  = RLP(["anr-v1", seq_be8, k₁, v₁, …])   # all kv pairs sorted
hash     = keccak256(content)
sig      = secp256k1_sign(hash, private_key)        # compact 64-byte r‖s
pub      = secp256k1_pubkey(private_key)            # 33-byte compressed
```

Verification:

```
content  = RLP(["anr-v1", record.seq_be8, record.kv…])
hash     = keccak256(content)
pub      = record.kv["secp256k1"]
valid    = secp256k1_verify(hash, sig=record.signature, pubkey=pub)
```

---

## Relationship to ENR (EIP-778)

| Property | ENR (EIP-778) | ANR |
|---|---|---|
| Purpose | Ethereum node identity | Agent identity |
| Wire format | RLP list | RLP list (identical structure) |
| Signing | secp256k1 / keccak256 | secp256k1 / keccak256 |
| Text form | `enr:<base64url>` | `anr:<base64url>` |
| Max size | 300 bytes | 512 bytes |
| Key ordering | Lexicographic | Lexicographic |
| Sequence number | uint64 | uint64 |
| Domain separator | none (v4 scheme) | `"anr-v1"` |
| Custom keys | Arbitrary | `a.*` namespace |

ANR is **intentionally a superset** of ENR. An ENR-aware parser can read the network keys from an ANR record without modification. The `a.*` keys will simply be ignored.

---

## Building an ANR

### TypeScript

```typescript
import { AnrBuilder } from './anr/anr';
import { secp256k1 }  from 'ethereum-cryptography/secp256k1';

const privateKey = secp256k1.utils.randomPrivateKey();

const record = new AnrBuilder()
  .setSeq(1n)
  .setAgentId('borgkit://agent/0xABC')
  .setName('WeatherAgent')
  .setVersion('1.0.0')
  .setCapabilities(['getWeather', 'getForecast'])
  .setTags(['weather', 'data'])
  .setProto('http')
  .setAgentPort(8080)
  .setTls(false)
  .setIpv4(new Uint8Array([192, 168, 1, 10]))
  .sign(privateKey);

const text = encodeANRText(record);
// → "anr:enqFiW..."
```

### Python

```python
from anr.anr import AnrBuilder
import os

private_key = os.urandom(32)

record = (
    AnrBuilder()
    .seq(1)
    .agent_id('borgkit://agent/0xABC')
    .name('WeatherAgent')
    .version('1.0.0')
    .capabilities(['getWeather', 'getForecast'])
    .tags(['weather', 'data'])
    .proto('http')
    .agent_port(8080)
    .tls(False)
    .ipv4(bytes([192, 168, 1, 10]))
    .sign(private_key)
)

text = record.encode_text()
# → "anr:enqFiW..."
```

---

## Decoding and verifying an ANR

```python
from anr.anr import ANR

text    = "anr:enqFiW..."
record  = ANR.decode_text(text)
valid   = record.verify()          # True / False
parsed  = record.parsed()          # ParsedANR object

print(parsed.name)                 # "WeatherAgent"
print(parsed.capabilities)        # ['getWeather', 'getForecast']
print(parsed.agent_port)          # 8080
```

---

## Updating an ANR

When an agent changes its IP, port, capabilities, or any other field, it must:

1. Increment `seq` by at least 1
2. Update the relevant key-value pairs
3. Re-sign with the same private key
4. Republish the new record to the discovery layer

Peers that receive a record with a **lower or equal** `seq` than what they already have cached must discard it. This prevents replay attacks.

---

## ANR and local discovery

During **local network discovery** (before any P2P gossip or on-chain lookup), agents broadcast their ANR over:

- **mDNS** — zero-config LAN discovery (like Bonjour/Avahi)
- **UDP broadcast** on port `21337` (the default Borgkit discovery port)
- **DNS TXT records** — for DNS-based bootstrapping

The `anr:` string is small enough to fit in all three transports without fragmentation.

---

## Future: on-chain ANR anchoring

Because ANR uses the same secp256k1 keys as Ethereum, an ANR record can be anchored on-chain via **ERC-8004**:

```solidity
// ERC-8004 registry stores agent metadata URI and owner
registry.setAgentRecord(agentId, anrTextString);
```

This enables:
- **Trustless verification** — anyone can verify an ANR without contacting the agent
- **Revocation** — owner can invalidate a record by publishing a new one with a higher `seq`
- **Delegation** — ERC-8004 delegation rights map directly to ANR signing authority
