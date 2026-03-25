# Changelog

All notable changes to `sentrix-cli` are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
