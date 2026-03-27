# Version Management

## Current version: `0.1.0`

Borgkit follows **Semantic Versioning 2.0.0** (`MAJOR.MINOR.PATCH`).

| Segment | Increment when |
|---|---|
| `MAJOR` | Breaking change to `IAgent`, `AgentRequest`, `AgentResponse`, ANR wire format, or CLI commands |
| `MINOR` | New backward-compatible feature (new discovery adapter, new plugin, new CLI command) |
| `PATCH` | Bug fix, documentation update, internal refactor with no API change |

---

## Single source of truth

The version lives in **one place only**:

```
src/version.ts  →  export const VERSION = '0.1.0';
```

Everything else reads from it:

```typescript
import { VERSION } from './version';

program.version(VERSION);                  // CLI --version flag
writeConfig({ version: VERSION, … });     // borgkit.config.json
```

---

## Checking version compatibility

```typescript
import { isCompatible } from './version';

// True if the running CLI satisfies a minimum requirement
isCompatible('0.1.0')   // true  (exact match)
isCompatible('0.0.9')   // true  (running is newer)
isCompatible('0.2.0')   // false (running is older)
```

---

## CLI version commands

```bash
borgkit --version       # short: prints "0.1.0"
borgkit -v              # alias

borgkit version         # detailed build info:
#   Version    : 0.1.0
#   Build date : 2026-03-24
#   Node.js    : v20.11.0
#   Platform   : darwin arm64
```

---

## Release process

1. Update `src/version.ts` — change `VERSION`
2. Add an entry to `CHANGELOG.md`
3. Commit: `git commit -m "chore: bump version to X.Y.Z"`
4. Tag: `git tag vX.Y.Z`
5. Push: `git push origin main --tags`
6. Publish: `npm publish`

---

## Roadmap

| Version | Target | Scope |
|---|---|---|
| **0.1.0** | ✅ now | CLI scaffold, interfaces, ANR, LocalDiscovery, HttpDiscovery, LangGraph + ADK plugins |
| **0.2.0** | Q2 2026 | GossipDiscovery (P2P), mDNS bootstrapping, ANR broadcast over UDP |
| **0.3.0** | Q3 2026 | OnChainDiscovery (ERC-8004), ANR on-chain anchoring |
| **0.4.0** | Q3 2026 | AMP-3 Payments (stream, oneshot, subscription) |
| **0.5.0** | Q4 2026 | AMP-4 Delegation & multi-agent workflows |
| **1.0.0** | 2027 | Stable wire format, production-ready P2P mesh |
