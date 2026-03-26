/**
 * Libp2pDiscovery — fully P2P discovery backend for Sentrix.
 *
 * Architecture:
 *   Transport   : QUIC (via @chainsafe/libp2p-quic) — no TCP listener opened
 *   Routing     : Kademlia DHT (custom /sentrix/kad/1.0.0 protocol)
 *   Local LAN   : mDNS (optional, default on)
 *   NAT         : DCUtR hole punching + circuit-relay-v2 fallback
 *   Identity    : secp256k1 keypair from ANR — same key, same PeerId
 *
 * Capability discovery uses DHT provider records:
 *   SHA256("sentrix:cap:<capability>") → CIDv1 → dht.provide() / findProviders()
 *
 * Full DiscoveryEntry is stored as a signed JSON envelope:
 *   SHA256("sentrix:anr:<agentId>") → key → dht.put() / get()
 *
 * A reverse PeerId→agentId mapping is also stored so findProviders() results
 * can be resolved to full entries.
 *
 * Usage:
 *   const discovery = await Libp2pDiscovery.create({ privateKey: myKey });
 *   await discovery.register(entry);
 *   const peers = await discovery.query('web_search');
 *   await discovery.stop();
 *
 * @see docs/libp2p.md for full architecture documentation.
 */

import { createLibp2p, type Libp2p } from 'libp2p';
import { quic }                       from '@chainsafe/libp2p-quic';
import { circuitRelayTransport }      from '@libp2p/circuit-relay-v2';
import { identify }                   from '@libp2p/identify';
import { kadDHT, type KadDHT }        from '@libp2p/kad-dht';
import { mdns }                       from '@libp2p/mdns';
import { dcutr }                      from '@libp2p/dcutr';
import { bootstrap }                  from '@libp2p/bootstrap';
import { toString, fromString }       from 'uint8arrays';
import type { PeerId }                from '@libp2p/interface';

import { IAgentDiscovery, DiscoveryEntry } from '../interfaces/IAgentDiscovery';
import { capabilityCid, anrDhtKey, pidDhtKey } from './libp2p/DhtKeys';
import { encodeEnvelope, decodeEnvelope }       from './libp2p/EntryEnvelope';
import { peerIdFromAnrKey, publicKeyFromAnrKey } from './libp2p/PeerIdFromAnr';
import { keys }                                  from '@libp2p/crypto';

// ── Types ────────────────────────────────────────────────────────────────────

export interface Libp2pDiscoveryConfig {
  /**
   * 32-byte raw secp256k1 private key — the same key used to sign ANR records.
   * If omitted, an ephemeral key is generated (not recommended for production).
   */
  privateKey?: Uint8Array;

  /**
   * Multiaddrs this node will listen on.
   * Default: ['/ip4/0.0.0.0/udp/0/quic-v1']  (random UDP port)
   */
  listenAddresses?: string[];

  /**
   * Multiaddrs of known bootstrap peers (format: /ip4/.../udp/.../quic-v1/p2p/...)
   * Also reads from SENTRIX_BOOTSTRAP_PEERS env var (comma-separated).
   */
  bootstrapPeers?: string[];

  /**
   * How often to re-publish DHT records to keep them alive (milliseconds).
   * Default: 30_000 (30 seconds)
   */
  heartbeatIntervalMs?: number;

  /**
   * Whether to enable mDNS for local network discovery.
   * Default: true
   * Disable in environments where LAN broadcast is undesirable.
   */
  enableMdns?: boolean;

  /**
   * Set to true to operate in DHT client mode — the node participates in
   * discovery but does not store records for others.  Reduces resource usage
   * on constrained agents.
   * Default: false
   */
  dhtClientMode?: boolean;
}

interface LocalEntry {
  entry: DiscoveryEntry;
  seq:   number;
  publicKey: Uint8Array;
}

// ── Sentinel bootstrap peers for the public Sentrix network ──────────────────
// Replace these with real multiaddrs when production bootstrap nodes are deployed.
const SENTRIX_BOOTSTRAP_PEERS: string[] = [
  // '/ip4/bootstrap1.sentrix.io/udp/4001/quic-v1/p2p/12D3KooWXXX...',
];

// ── Libp2pDiscovery ───────────────────────────────────────────────────────────

export class Libp2pDiscovery implements IAgentDiscovery {
  private readonly node:    Libp2p;
  private readonly dht:     KadDHT;
  private readonly privKey: Uint8Array;
  private readonly pubKey:  Uint8Array;

  /** Locally registered agents: agentId → entry + seq + public key */
  private localEntries = new Map<string, LocalEntry>();

  /** Background heartbeat timers: agentId → NodeJS.Timer */
  private heartbeatTimers = new Map<string, ReturnType<typeof setInterval>>();

  /** Short-lived query cache: capability → { entries, expiresAt } */
  private queryCache = new Map<string, { entries: DiscoveryEntry[]; expiresAt: number }>();

  private constructor(node: Libp2p, dht: KadDHT, privKey: Uint8Array, pubKey: Uint8Array) {
    this.node    = node;
    this.dht     = dht;
    this.privKey = privKey;
    this.pubKey  = pubKey;
  }

  // ── Factory ─────────────────────────────────────────────────────────────────

  /**
   * Create and start a Libp2pDiscovery node.
   * This is an async factory because libp2p.start() is asynchronous.
   */
  static async create(config: Libp2pDiscoveryConfig = {}): Promise<Libp2pDiscovery> {
    // ── Keypair ──────────────────────────────────────────────────────────────
    let privKey: Uint8Array;
    let libp2pPrivKey: Awaited<ReturnType<typeof keys.generateKeyPairFromSeed>>;

    if (config.privateKey) {
      privKey      = config.privateKey;
      libp2pPrivKey = await keys.generateKeyPairFromSeed('secp256k1', privKey);
    } else {
      // Ephemeral key — generates random secp256k1 keypair
      libp2pPrivKey = await keys.generateKeyPair('secp256k1');
      privKey        = (libp2pPrivKey as any).raw ?? new Uint8Array(32); // fallback
      console.warn('[Libp2pDiscovery] Using ephemeral identity — set privateKey for production');
    }
    const pubKey = libp2pPrivKey.publicKey.raw;

    // ── Bootstrap peers ──────────────────────────────────────────────────────
    const envPeers    = process.env['SENTRIX_BOOTSTRAP_PEERS']?.split(',').filter(Boolean) ?? [];
    const allBootstrap = [
      ...(config.bootstrapPeers ?? []),
      ...envPeers,
      ...SENTRIX_BOOTSTRAP_PEERS,
    ];

    // ── libp2p node ──────────────────────────────────────────────────────────
    const peerDiscovery = [
      ...(config.enableMdns !== false ? [mdns()] : []),
      ...(allBootstrap.length > 0    ? [bootstrap({ list: allBootstrap })] : []),
    ];

    const node = await createLibp2p({
      privateKey: libp2pPrivKey,
      addresses: {
        listen: config.listenAddresses ?? ['/ip4/0.0.0.0/udp/0/quic-v1'],
      },
      transports: [
        quic(),
        // Circuit relay transport for NAT fallback (outbound connections only)
        circuitRelayTransport({ discoverRelays: 1 }),
      ],
      // QUIC handles TLS natively — no separate connectionEncrypter needed
      peerDiscovery,
      services: {
        identify: identify(),
        dht: kadDHT({
          protocol:   '/sentrix/kad/1.0.0',  // isolated from public IPFS DHT
          clientMode: config.dhtClientMode ?? false,
        }),
        dcutr:  dcutr(),
      },
    });

    await node.start();

    const dht = node.services['dht'] as unknown as KadDHT;

    console.log(
      `[Libp2pDiscovery] Started — PeerId: ${node.peerId}  ` +
      `Addrs: ${node.getMultiaddrs().map(String).join(', ') || '(none yet)'}`,
    );

    return new Libp2pDiscovery(node, dht, privKey, pubKey);
  }

  /**
   * Return the local node's PeerId and first multiaddr, or null if not started.
   * Used by ExampleAgent to populate the `multiaddr` field in ANR records.
   */
  getNodeInfo(): { peerId: string; multiaddr: string } | null {
    const peerId = this.node.peerId.toString();
    const addrs  = this.node.getMultiaddrs();
    if (!peerId) return null;
    return { peerId, multiaddr: addrs.length > 0 ? addrs[0].toString() : '' };
  }

  /** Gracefully stop the libp2p node and clear all timers. */
  async stop(): Promise<void> {
    for (const timer of this.heartbeatTimers.values()) clearInterval(timer);
    this.heartbeatTimers.clear();
    await this.node.stop();
    console.log('[Libp2pDiscovery] Stopped');
  }

  // ── IAgentDiscovery ──────────────────────────────────────────────────────────

  async register(entry: DiscoveryEntry): Promise<void> {
    const seq = (this.localEntries.get(entry.agentId)?.seq ?? 0) + 1;

    // Persist locally first so we can respond even if DHT is not yet connected
    this.localEntries.set(entry.agentId, { entry, seq, publicKey: this.pubKey });

    // Publish to DHT (fire-and-forget with error logging)
    this.publishToDht(entry, seq).catch(err =>
      console.warn(`[Libp2pDiscovery] DHT publish pending (${entry.agentId}): ${err.message}`)
    );

    // Start heartbeat refresh timer
    this.startHeartbeat(entry.agentId, entry, seq);

    console.log(`[Libp2pDiscovery] Registered: ${entry.agentId}  caps=${entry.capabilities}`);
  }

  async unregister(agentId: string): Promise<void> {
    const timer = this.heartbeatTimers.get(agentId);
    if (timer) { clearInterval(timer); this.heartbeatTimers.delete(agentId); }
    this.localEntries.delete(agentId);
    // DHT provider records expire naturally; we cannot actively revoke them.
    // Mark the value record as unhealthy so peers stop using it.
    console.log(`[Libp2pDiscovery] Unregistered: ${agentId}`);
  }

  async query(capability: string): Promise<DiscoveryEntry[]> {
    // 1. Check short-lived cache (10s TTL)
    const cached = this.queryCache.get(capability);
    if (cached && cached.expiresAt > Date.now()) return cached.entries;

    // 2. Check local entries first (zero network latency)
    const local = [...this.localEntries.values()]
      .filter(e => e.entry.capabilities.includes(capability) && e.entry.health.status !== 'unhealthy')
      .map(e => e.entry);

    // 3. Query DHT for remote providers
    const remote: DiscoveryEntry[] = [];
    try {
      const capCid = await capabilityCid(capability);

      for await (const event of this.dht.findProviders(capCid, { signal: AbortSignal.timeout(5_000) })) {
        if ((event as any).name !== 'PROVIDER') continue;
        const providerId: PeerId = (event as any).peer?.id;
        if (!providerId) continue;
        if (providerId.equals(this.node.peerId)) continue; // skip self

        const entry = await this.fetchEntryForPeer(providerId);
        if (entry && entry.health.status !== 'unhealthy') remote.push(entry);
      }
    } catch (err: any) {
      console.warn(`[Libp2pDiscovery] DHT findProviders failed for '${capability}': ${err.message}`);
    }

    // 4. Merge, deduplicate by agentId
    const seen = new Set<string>();
    const all: DiscoveryEntry[] = [];
    for (const e of [...local, ...remote]) {
      if (!seen.has(e.agentId)) { seen.add(e.agentId); all.push(e); }
    }

    // 5. Cache for 10 seconds
    this.queryCache.set(capability, { entries: all, expiresAt: Date.now() + 10_000 });
    return all;
  }

  async listAll(): Promise<DiscoveryEntry[]> {
    // Local entries are authoritative; remote is best-effort
    const local = [...this.localEntries.values()].map(e => e.entry);
    return local;
    // Note: a full DHT walk for remote agents is expensive and eventually-consistent.
    // In production, use a dedicated gossip channel or a list built from accumulated
    // query results. See docs/libp2p.md § listAll.
  }

  async heartbeat(agentId: string): Promise<void> {
    const local = this.localEntries.get(agentId);
    if (!local) {
      console.warn(`[Libp2pDiscovery] heartbeat called for unknown agent: ${agentId}`);
      return;
    }
    const newSeq   = local.seq + 1;
    const newEntry = {
      ...local.entry,
      health: {
        ...local.entry.health,
        status: 'healthy' as const,
        lastHeartbeat: new Date().toISOString(),
      },
    };
    this.localEntries.set(agentId, { ...local, entry: newEntry, seq: newSeq });
    await this.publishToDht(newEntry, newSeq);
  }

  // ── Private helpers ──────────────────────────────────────────────────────────

  /** Publish a DiscoveryEntry + all its capability keys to the DHT. */
  private async publishToDht(entry: DiscoveryEntry, seq: number): Promise<void> {
    const encoded   = encodeEnvelope(entry, seq, this.privKey);
    const valueKey  = await anrDhtKey(entry.agentId);
    const pidKey    = pidDhtKey(this.node.peerId.toString());

    await Promise.all([
      // (a) Store the full DiscoveryEntry envelope
      this.dht.put(valueKey, encoded),

      // (b) Store the PeerId → agentId reverse mapping
      this.dht.put(pidKey, new TextEncoder().encode(entry.agentId)),

      // (c) Announce as provider for every capability
      ...entry.capabilities.map(async cap => {
        const cid = await capabilityCid(cap);
        await this.dht.provide(cid);
      }),
    ]);
  }

  /** Fetch a DiscoveryEntry for a remote peer via the DHT. */
  private async fetchEntryForPeer(peerId: PeerId): Promise<DiscoveryEntry | null> {
    try {
      // Step 1: resolve PeerId → agentId
      const pidKey   = pidDhtKey(peerId.toString());
      const agentIdRaw = await this.dhtGet(pidKey);
      if (!agentIdRaw) return null;

      const agentId  = new TextDecoder().decode(agentIdRaw);

      // Step 2: resolve agentId → DiscoveryEntry envelope
      const valueKey = await anrDhtKey(agentId);
      const raw      = await this.dhtGet(valueKey);
      if (!raw) return null;

      const decoded  = decodeEnvelope(raw);
      return decoded?.entry ?? null;
    } catch {
      return null;
    }
  }

  /** Wraps dht.get() to consume the async iterator and return the first value. */
  private async dhtGet(key: Uint8Array): Promise<Uint8Array | null> {
    for await (const event of this.dht.get(key, { signal: AbortSignal.timeout(3_000) })) {
      if ((event as any).name === 'VALUE') {
        return (event as any).value as Uint8Array;
      }
    }
    return null;
  }

  /** Start a background timer that re-publishes DHT records at heartbeatIntervalMs. */
  private startHeartbeat(
    agentId: string,
    _entry: DiscoveryEntry,
    _seq: number,
    intervalMs = 30_000,
  ): void {
    if (this.heartbeatTimers.has(agentId)) {
      clearInterval(this.heartbeatTimers.get(agentId)!);
    }
    const timer = setInterval(
      () => this.heartbeat(agentId).catch(e =>
        console.warn(`[Libp2pDiscovery] heartbeat error for ${agentId}: ${e.message}`)
      ),
      intervalMs,
    );
    this.heartbeatTimers.set(agentId, timer);
  }
}
