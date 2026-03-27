# Agent Identity

Borgkit agents need an identity to:
- Sign ANR records (Agent Name Records)
- Authenticate agent-to-agent calls
- Derive a stable libp2p PeerId for P2P discovery
- Optionally anchor ownership on-chain

**ERC-8004 on-chain registration is entirely optional.** You get a fully functional, cryptographically signed identity using only a local private key — no wallet, no gas, no tokens required.

---

## Identity modes

| Mode | Requires wallet? | On-chain? | Best for |
|------|-----------------|-----------|----------|
| `anonymous` | no | no | Local dev, throwaway agents |
| `local` | no | no | **Default** — persistent key auto-created in `~/.borgkit/keystore/` |
| `env` | no | no | Containers, cloud, CI/CD (12-factor) |
| `raw` | no | no | Bring-your-own key (from secret manager etc.) |
| `erc8004` | yes | optional | Production — verifiable on-chain ownership |

All keyed modes (local, env, raw, erc8004) use the **same secp256k1 private key** for:
- Signing ANR records
- Deriving the agent's Ethereum address (owner)
- Deriving the libp2p PeerId (`Secp256k1Keypair`)

---

## Quick start

### Python

```python
# Simplest: auto-creates key in ~/.borgkit/keystore/my-agent.key
from identity.provider import LocalKeystoreIdentity

identity = LocalKeystoreIdentity(name="my-agent")
print(identity.agent_id())   # borgkit://agent/0xAbCd...
print(identity.owner())      # 0xAbCd...  (Ethereum address)

# Use with PluginConfig
from plugins.base import PluginConfig
config = PluginConfig(**identity.to_plugin_config_fields(), port=8080)
```

```python
# Container / cloud: key from environment variable
from identity.provider import EnvKeyIdentity
# export BORGKIT_AGENT_KEY=0x<32-byte-hex>

identity = EnvKeyIdentity()
```

```python
# No identity (dev-only)
from identity.provider import AnonymousIdentity

identity = AnonymousIdentity(name="ephemeral-agent")
```

```python
# On-chain (optional) — same interface, just adds register_on_chain()
from identity.provider import ERC8004Identity
import os

identity = ERC8004Identity(
    private_key_hex=os.environ["WALLET_KEY"],
    chain_id=8453,                          # Base mainnet
    contract_address="0x...",
    rpc_url="https://mainnet.base.org",
)
# Optionally publish ANR on-chain (requires gas):
tx_hash = await identity.register_on_chain(anr_text)
```

### TypeScript

```typescript
import { LocalKeystoreIdentity } from './identity';

const identity = new LocalKeystoreIdentity('my-agent');
console.log(identity.agentId());  // borgkit://agent/0xAbCd...
console.log(identity.owner());    // 0xAbCd...

// Use with wrapped agent config
const { agentId, owner, signingKey } = identity.toPluginConfigFields();
```

```typescript
// Container / cloud
import { EnvKeyIdentity } from './identity';
// BORGKIT_AGENT_KEY=0x<32-byte-hex>
const identity = new EnvKeyIdentity();
```

```typescript
// On-chain (optional)
import { ERC8004Identity } from './identity';

const identity = new ERC8004Identity({
  privateKeyHex: process.env.WALLET_KEY!,
  chainId: 8453,
  contractAddress: '0x...',
  rpcUrl: 'https://mainnet.base.org',
});
await identity.registerOnChain(anrText);  // optional
```

---

## How identity flows through Borgkit

```
Private key (secp256k1, 32 bytes)
  │
  ├─── Ethereum address derivation ──→ agent_id / owner fields
  │                                    (borgkit://agent/0xAbCd...)
  │
  ├─── ANR signing ─────────────────→ signed ANR text (anr:...)
  │                                    published to discovery layer
  │
  └─── libp2p PeerId derivation ────→ Secp256k1Keypair
                                       P2P node identity (libp2p)
```

The same 32-byte key is the single source of truth for all three derived identities.

---

## Keystore file format

`LocalKeystoreIdentity` stores a plain-text 32-byte hex key:

```
~/.borgkit/keystore/
└── my-agent.key    (chmod 0600)
    # Contents: 64 lowercase hex chars, no 0x prefix
    # e.g.: a1b2c3d4e5f6...
```

- File is created on first use with `chmod 0600`
- Keystore directory is created with `chmod 0700`
- Back this file up — if lost, the agent gets a new identity

---

## Environment variable format

```bash
# 32-byte hex, with or without 0x prefix
export BORGKIT_AGENT_KEY=0xa1b2c3d4e5f6...  # with 0x
export BORGKIT_AGENT_KEY=a1b2c3d4e5f6...    # also accepted
```

---

## When to use ERC-8004

Use ERC-8004 when you need:
- **Verifiable on-chain ownership** — third parties can query the registry contract to confirm an agent is owned by a specific wallet
- **Wallet-gated capabilities** — smart contracts that check agent registration before execution
- **Token-curated registries** — staking to list agents in curated discovery

You do **not** need ERC-8004 for:
- Local or enterprise deployments
- Agents that authenticate via ANR signatures only
- Development and testing
- Agents where the owner is a developer/company rather than a wallet

---

## Generating a new key

```bash
# Python (one-liner)
python -c "import secrets; print(secrets.token_hex(32))"

# Node.js
node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"

# Or let Borgkit create one automatically:
python -c "from identity.provider import LocalKeystoreIdentity; LocalKeystoreIdentity('my-agent')"
```

---

## Security notes

- Never commit private keys to source control
- In production, use `EnvKeyIdentity` with keys injected via secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)
- `LocalKeystoreIdentity` is suitable for single-machine persistent agents; use `EnvKeyIdentity` for containerised deployments
- ERC-8004 wallets should use hardware signing (Ledger, Trezor) or KMS-backed wallets in high-value scenarios
