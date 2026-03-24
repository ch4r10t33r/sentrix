{
  "name": "{{PROJECT_NAME}}",
  "version": "0.1.0",
  "description": "Sentrix agent project — ERC-8004 compliant",
  "scripts": {
    "dev":   "ts-node agents/ExampleAgent.ts",
    "build": "tsc",
    "start": "node dist/agents/ExampleAgent.js",
    "test":  "jest"
  },
  "dependencies": {
    "express": "^4.18.2",
    "ws":      "^8.16.0",

    "libp2p":                    "^2.4.0",
    "@chainsafe/libp2p-quic":    "^0.2.0",
    "@libp2p/bootstrap":         "^10.0.0",
    "@libp2p/circuit-relay-v2":  "^3.0.0",
    "@libp2p/crypto":            "^4.1.0",
    "@libp2p/dcutr":             "^9.0.0",
    "@libp2p/identify":          "^3.0.0",
    "@libp2p/kad-dht":           "^12.1.0",
    "@libp2p/mdns":              "^10.0.0",
    "@libp2p/peer-id":           "^4.1.0",
    "multiformats":              "^13.1.0",
    "uint8arrays":               "^5.1.0",
    "@noble/curves":             "^1.4.0",
    "@noble/hashes":             "^1.4.0"
  },
  "devDependencies": {
    "@types/express":  "^4.17.21",
    "@types/node":     "^20.11.0",
    "@types/ws":       "^8.5.10",
    "ts-node":         "^10.9.2",
    "typescript":      "^5.3.3"
  },
  "engines": {
    "node": ">=20.0.0"
  }
}
