# Changelog

All notable changes to `inai-cli` are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/ch4r10t33r/inai/compare/v0.2.0...v0.3.0) (2026-03-27)

### Features

* make libp2p the default discovery mode ([80dc7bb](https://github.com/ch4r10t33r/inai/commit/80dc7bb52fe1b7f8d1d5b0be1d28e6988146e33a))

## [0.2.0](https://github.com/ch4r10t33r/inai/compare/v0.1.1...v0.2.0) (2026-03-27)

### Features

* **scaffold:** inject flag-aware deps into package.json / Cargo.toml / build.zig.zon ([de4a1f1](https://github.com/ch4r10t33r/inai/commit/de4a1f1e06e3590ec365b4a75c9b09e038654ddc))

## [0.1.1](https://github.com/ch4r10t33r/inai/compare/v0.1.0...v0.1.1) (2026-03-27)

### Bug Fixes

* **ci:** add sleep between npm publishes to avoid 409 registry race ([0c436b3](https://github.com/ch4r10t33r/inai/commit/0c436b3a36c5fb08541e2f4471042fb7d394afb2))

## [0.1.0](https://github.com/ch4r10t33r/inai/compare/v0.0.0...v0.1.0) (2026-03-27)

### Features

* **cli:** add inai upgrade command ([f7e02d2](https://github.com/ch4r10t33r/inai/commit/f7e02d29bd018af2a0e6ce73efadaf7b0cae85db))

### Bug Fixes

* fully regenerate package-lock.json from scratch (nested deps still had 0.1.0 from prior sed) ([0b713a9](https://github.com/ch4r10t33r/inai/commit/0b713a9a8fd97d7acd1d4fa681d95dbb520a8fcc))
* regenerate package-lock.json after version reset (sed had clobbered all dep versions to 0.1.0) ([a199e3e](https://github.com/ch4r10t33r/inai/commit/a199e3ecbf7a4b1813ce1e2d7e398cb51bbff00a))
* rename bin/sentrix.js → bin/inai.js to match package.json bin field ([99c1958](https://github.com/ch4r10t33r/inai/commit/99c1958661bcc7b87a8c3cdadc059414810351a0))
* reset package versions to 0.1.0 for initial inai release ([94c4bb8](https://github.com/ch4r10t33r/inai/commit/94c4bb82f23451cbef7d5b47880a2309316a1449))
* **scripts:** sync-release-versions exits 1 when Cargo.toml already at target version ([2609639](https://github.com/ch4r10t33r/inai/commit/2609639a2c6751439245b35ea47ae51144d19de8))

## [1.22.0](https://github.com/ch4r10t33r/inai/compare/v1.21.0...v1.22.0) (2026-03-27)

### Features

* **cli:** add inai upgrade command ([f7e02d2](https://github.com/ch4r10t33r/inai/commit/f7e02d29bd018af2a0e6ce73efadaf7b0cae85db))

## [1.22.1](https://github.com/ch4r10t33r/inai/compare/v1.22.0...v1.22.1) (2026-03-27)

### Bug Fixes

* fully regenerate package-lock.json from scratch (nested deps still had 0.1.0 from prior sed) ([0b713a9](https://github.com/ch4r10t33r/inai/commit/0b713a9a8fd97d7acd1d4fa681d95dbb520a8fcc))
* regenerate package-lock.json after version reset (sed had clobbered all dep versions to 0.1.0) ([a199e3e](https://github.com/ch4r10t33r/inai/commit/a199e3ecbf7a4b1813ce1e2d7e398cb51bbff00a))
* rename bin/sentrix.js → bin/inai.js to match package.json bin field ([99c1958](https://github.com/ch4r10t33r/inai/commit/99c1958661bcc7b87a8c3cdadc059414810351a0))

# Changelog

All notable changes to `sentrix-cli` are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.22.0](https://github.com/ch4r10t33r/sentrix/compare/v1.21.0...v1.22.0) (2026-03-27)

### Features

* **mpp:** Machine Payments Protocol plugin — TypeScript, Rust, Zig ([d6ef275](https://github.com/ch4r10t33r/sentrix/commit/d6ef27557c81b24d43becc22a13d9c61bc8c260c))

## [1.21.0](https://github.com/ch4r10t33r/sentrix/compare/v1.20.0...v1.21.0) (2026-03-27)

### Features

* **didcomm:** DIDComm v2 encrypted messaging — TypeScript, Rust, Zig ([6698342](https://github.com/ch4r10t33r/sentrix/commit/669834200b6a9c99dcd0df131df5643bb0b4b892))

## [1.20.0](https://github.com/ch4r10t33r/sentrix/compare/v1.19.0...v1.20.0) (2026-03-27)

### Features

* **cli:** add sentrix scaffold command ([e2bd80f](https://github.com/ch4r10t33r/sentrix/commit/e2bd80fc162702474ed5be0d6dad75926bfba194))

## [1.19.0](https://github.com/ch4r10t33r/sentrix/compare/v1.18.0...v1.19.0) (2026-03-27)

### Features

* **zig:** full Kademlia DHT discovery — pure Zig UDP/JSON implementation ([b721d06](https://github.com/ch4r10t33r/sentrix/commit/b721d06465695e913d8035f406b91e8ae6b104f2))

## [1.18.0](https://github.com/ch4r10t33r/sentrix/compare/v1.17.0...v1.18.0) (2026-03-27)

### Features

* **templates:** MCP bridge for Rust and Zig (both directions) ([d1dcf61](https://github.com/ch4r10t33r/sentrix/commit/d1dcf61fb29a65f6f487d688379f2d4d37660a5a))

## [1.17.0](https://github.com/ch4r10t33r/sentrix/compare/v1.16.0...v1.17.0) (2026-03-26)

### Features

* **templates:** add missing plugins and addons for Rust and Zig ([99f52a4](https://github.com/ch4r10t33r/sentrix/commit/99f52a4e9ca3f986592d82ae510c4d3e3e14722e))

## [1.16.0](https://github.com/ch4r10t33r/sentrix/compare/v1.15.3...v1.16.0) (2026-03-26)

### Features

* **plugins:** add LangGraph, Google ADK, and CrewAI plugins for TypeScript, Rust, and Zig ([5e54692](https://github.com/ch4r10t33r/sentrix/commit/5e5469292da8eb2097c6179c0e838d87f522ca8d))

## [1.15.3](https://github.com/ch4r10t33r/sentrix/compare/v1.15.2...v1.15.3) (2026-03-26)

### Bug Fixes

* normalize package.json bin path and repository url for npm ([462ed26](https://github.com/ch4r10t33r/sentrix/commit/462ed26134656dad2c9a8cc6605d4eeaacf6f5f8))

## [1.15.2](https://github.com/ch4r10t33r/sentrix/compare/v1.15.1...v1.15.2) (2026-03-26)

### Bug Fixes

* **release:** sync Cargo/npm versions and reliable CLI trigger ([d259762](https://github.com/ch4r10t33r/sentrix/commit/d259762cb136fefde2c91dac60f5861149cbaca1))

## [1.15.1](https://github.com/ch4r10t33r/sentrix/compare/v1.15.0...v1.15.1) (2026-03-26)


### Bug Fixes

* extend logo background to full card so text is visible in light mode ([379a3de](https://github.com/ch4r10t33r/sentrix/commit/379a3de98b8b0fe79b8a9913c6e6920f132b7063))
* use absolute URL for logo so it renders on npm and other hosts ([8bfc7c6](https://github.com/ch4r10t33r/sentrix/commit/8bfc7c63cbe5c1cc5d597efa78d05da16d301f3d))

# [1.15.0](https://github.com/ch4r10t33r/sentrix/compare/v1.14.2...v1.15.0) (2026-03-26)


### Features

* add Sentrix logo (P2P agent mesh, hex topology, DID-native) ([039fb3c](https://github.com/ch4r10t33r/sentrix/commit/039fb3c78b01052893c4a2bc199f29328d03e0d8))

## [1.14.2](https://github.com/ch4r10t33r/sentrix/compare/v1.14.1...v1.14.2) (2026-03-26)


### Bug Fixes

* install.sh falls back to ~/.local/bin when piped without TTY ([db1ebb9](https://github.com/ch4r10t33r/sentrix/commit/db1ebb95d7b5958a21a670d902439d8f3d4cd03a))

## [1.14.1](https://github.com/ch4r10t33r/sentrix/compare/v1.14.0...v1.14.1) (2026-03-26)


### Bug Fixes

* chain release-cli.yml from release.yml to fix missing release binaries ([c24fb19](https://github.com/ch4r10t33r/sentrix/commit/c24fb1946b4a6965e7709307f9662fee1243505f))

# [1.14.0](https://github.com/ch4r10t33r/sentrix/compare/v1.13.0...v1.14.0) (2026-03-26)


### Features

* add single-line curl installer with platform auto-detection ([f8ce930](https://github.com/ch4r10t33r/sentrix/commit/f8ce9303246898fab935073495bc9cf30793bf1a))

# [1.13.0](https://github.com/ch4r10t33r/sentrix/compare/v1.12.0...v1.13.0) (2026-03-26)


### Bug Fixes

* resolve all clippy warnings in sentrix-cli ([11a4339](https://github.com/ch4r10t33r/sentrix/commit/11a4339d3c4396f1e9b8681473fc1b87b3acd086))
* use package.json as source of truth for reported CLI version ([823e27a](https://github.com/ch4r10t33r/sentrix/commit/823e27ab1ff893862ac48d2369226c376f276e96))


### Features

* Rust CLI binary with npm platform-package distribution ([5cb97f1](https://github.com/ch4r10t33r/sentrix/commit/5cb97f1093f4ce05d1793337e5c7a7d901b999e2))

# Changelog

All notable changes to `sentrix-cli` are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.12.0](https://github.com/ch4r10t33r/sentrix/compare/v1.11.0...v1.12.0) (2026-03-26)

### Features

* default port 6174, multiaddr always populated, libp2p port documented ([fa4e8e6](https://github.com/ch4r10t33r/sentrix/commit/fa4e8e68441e172d599f66fbe62b7898e6708b9e))

## [1.11.0](https://github.com/ch4r10t33r/sentrix/compare/v1.10.0...v1.11.0) (2026-03-26)

### Features

* implement sign_message, env-driven discovery, and permission checks ([c8c265b](https://github.com/ch4r10t33r/sentrix/commit/c8c265bf02957eda81957bbb55e4d419293e73fe))

## [1.10.0](https://github.com/ch4r10t33r/sentrix/compare/v1.9.0...v1.10.0) (2026-03-26)

### Features

* add HTTP server and plugin system for Rust and Zig templates ([b8a435e](https://github.com/ch4r10t33r/sentrix/commit/b8a435e8d68f118fe870c181803e79ad30e5c678))
* add sentrix test/inspect CLI commands and complete TypeScript plugin stubs ([88f9221](https://github.com/ch4r10t33r/sentrix/commit/88f92218dd8dd4f0f7573222bcf42006f9421289))
* implement OnChainDiscovery + fix libp2p DHT signing and peer_id gap ([13a550a](https://github.com/ch4r10t33r/sentrix/commit/13a550a93ca8ce2740d27eca48385d73996671c9))

## [1.9.0](https://github.com/ch4r10t33r/sentrix/compare/v1.8.0...v1.9.0) (2026-03-26)

### Features

* add libp2p transport (TypeScript, Rust + C FFI for Python/Zig) ([d884a04](https://github.com/ch4r10t33r/sentrix/commit/d884a047c2f7a96ee6077d6810d5f38f810c609b))

## [1.8.0](https://github.com/ch4r10t33r/sentrix/compare/v1.7.0...v1.8.0) (2026-03-26)

### Features

* add SSE streaming to agents via POST /invoke/stream ([99bc801](https://github.com/ch4r10t33r/sentrix/commit/99bc8016d6e28ea3fd7174ff357f10014da67857))

## [1.7.0](https://github.com/ch4r10t33r/sentrix/compare/v1.6.0...v1.7.0) (2026-03-25)

### Features

* **openai:** OpenAI Agents SDK plugin + README refresh ([c2a6f13](https://github.com/ch4r10t33r/sentrix/commit/c2a6f13bf2f9c111a30dbab61957f75db85282df))

## [1.6.0](https://github.com/ch4r10t33r/sentrix/compare/v1.5.0...v1.6.0) (2026-03-25)

### Features

* **mcp:** two-way MCP bridge — MCPPlugin + serve_as_mcp ([f65d1f7](https://github.com/ch4r10t33r/sentrix/commit/f65d1f75584b445f60d9bec20b14c43d2a00744e))

## [1.5.0](https://github.com/ch4r10t33r/sentrix/compare/v1.4.0...v1.5.0) (2026-03-25)

### Features

* **server:** built-in HTTP server for sentrix run ([f2d5673](https://github.com/ch4r10t33r/sentrix/commit/f2d56736021c6e1472d8d70f2bed3a71c9db9ba0))

## [1.4.0](https://github.com/ch4r10t33r/sentrix/compare/v1.3.0...v1.4.0) (2026-03-25)

### Features

* **examples:** cross-framework A2A example — Google ADK + CrewAI ([f475462](https://github.com/ch4r10t33r/sentrix/commit/f4754628c7d82cc31a4e63bb30b4cdfd30e0e7c1))

## [1.3.0](https://github.com/ch4r10t33r/sentrix/compare/v1.2.0...v1.3.0) (2026-03-25)

### Features

* **mesh:** heartbeat, capability exchange (handshake), and gossip protocols ([971c053](https://github.com/ch4r10t33r/sentrix/commit/971c053534d47c42fd9cd45f4b816465baf5e62a))
* **mesh:** startup banner on register_discovery(); rewrite docs/interfaces.md ([b91d713](https://github.com/ch4r10t33r/sentrix/commit/b91d71336824f60c2a38936160a9d688fd5b2d74))

### Bug Fixes

* **cli:** dynamic import ora to fix ERR_REQUIRE_ESM on Node 18 ([ac2e4ce](https://github.com/ch4r10t33r/sentrix/commit/ac2e4cedc261c5a47482e28469ca400a63b6e618))
* **deps:** downgrade chalk/ora/inquirer to last CJS-compatible versions ([fd42d9f](https://github.com/ch4r10t33r/sentrix/commit/fd42d9fd82bf9954f1616a9e1a4f462a19c27c69))

## [1.2.0](https://github.com/ch4r10t33r/sentrix/compare/v1.1.0...v1.2.0) (2026-03-25)

### Features

* add IAgentClient with lookup and A2A interaction across all languages ([dfb21f9](https://github.com/ch4r10t33r/sentrix/commit/dfb21f9b65a72a883f2c49022d60f969d6681b01))

## [1.1.0](https://github.com/ch4r10t33r/sentrix/compare/v1.0.0...v1.1.0) (2026-03-25)

### Features

* expose get_anr() and get_peer_id() on IAgent across all languages ([2bf2073](https://github.com/ch4r10t33r/sentrix/commit/2bf2073c52e9c4f5164cfb60c12b9ff9425885bb))

## 1.0.0 (2026-03-25)

### Features

* add --framework option to sentrix create with 7 framework adapters ([0d0a48d](https://github.com/ch4r10t33r/sentrix/commit/0d0a48d8ad80911fb63f821f66969158f6660cc2))
* add libp2p P2P discovery backend (QUIC + Kademlia DHT) ([4be9a10](https://github.com/ch4r10t33r/sentrix/commit/4be9a1006d63d747ff01e5f33f3bf1f67b91d4d6))
* add version management (v0.1.0) and comprehensive docs ([0e42fac](https://github.com/ch4r10t33r/sentrix/commit/0e42fac4ab35cb500803c2c88d70f2d9d4bf7bfe))
* auto-install framework dependencies on sentrix create ([caf0060](https://github.com/ch4r10t33r/sentrix/commit/caf0060c6e365a0446a7f0ca092debc63bb41684))
* DID-native identity, x402 payment add-on, optional ERC-8004 ([9b26cc6](https://github.com/ch4r10t33r/sentrix/commit/9b26cc6b04f076e28e6c875aa0acd8eaa0600c22))

### Bug Fixes

* align agent metadata types and docs with current codebase ([a0abf0e](https://github.com/ch4r10t33r/sentrix/commit/a0abf0ecbf42fa5b7963115fb84428c1b1ce835e))
* **release:** remove trailing comma in .releaserc.json (invalid JSON) ([119d10c](https://github.com/ch4r10t33r/sentrix/commit/119d10c8588be05e639dc913ef4ac18dcc3f06e2))
* **release:** scope package to [@ch4r10teer41](https://github.com/ch4r10teer41) org and set publishConfig access public ([ec24965](https://github.com/ch4r10t33r/sentrix/commit/ec24965d7ab80742a8104c35ca843c53422c0d50))
* replace ASCII architecture diagram with markdown table for clean GitHub rendering ([6ec5916](https://github.com/ch4r10t33r/sentrix/commit/6ec5916caa3457674e00376982bb522527ef48c8))

## 1.0.0 (2026-03-25)

### Features

* add --framework option to sentrix create with 7 framework adapters ([0d0a48d](https://github.com/ch4r10t33r/sentrix/commit/0d0a48d8ad80911fb63f821f66969158f6660cc2))
* add libp2p P2P discovery backend (QUIC + Kademlia DHT) ([4be9a10](https://github.com/ch4r10t33r/sentrix/commit/4be9a1006d63d747ff01e5f33f3bf1f67b91d4d6))
* add version management (v0.1.0) and comprehensive docs ([0e42fac](https://github.com/ch4r10t33r/sentrix/commit/0e42fac4ab35cb500803c2c88d70f2d9d4bf7bfe))
* auto-install framework dependencies on sentrix create ([caf0060](https://github.com/ch4r10t33r/sentrix/commit/caf0060c6e365a0446a7f0ca092debc63bb41684))
* DID-native identity, x402 payment add-on, optional ERC-8004 ([9b26cc6](https://github.com/ch4r10t33r/sentrix/commit/9b26cc6b04f076e28e6c875aa0acd8eaa0600c22))

### Bug Fixes

* align agent metadata types and docs with current codebase ([a0abf0e](https://github.com/ch4r10t33r/sentrix/commit/a0abf0ecbf42fa5b7963115fb84428c1b1ce835e))
* replace ASCII architecture diagram with markdown table for clean GitHub rendering ([6ec5916](https://github.com/ch4r10t33r/sentrix/commit/6ec5916caa3457674e00376982bb522527ef48c8))

<!-- semantic-release will prepend new entries above this line -->
