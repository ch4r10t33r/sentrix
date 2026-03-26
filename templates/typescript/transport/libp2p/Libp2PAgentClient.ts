/**
 * Libp2PAgentClient — IAgentClient implementation over libp2p streams.
 *
 * Replaces the HTTP transport (AgentClient) for peer-to-peer communication.
 * Falls back to AgentClient (HTTP) for entries that carry no peerId/multiaddr.
 *
 * Transport selection:
 *   entry.network.peerId present  → use libp2p stream
 *   entry.network.peerId absent   → fall back to HTTP AgentClient
 */
import * as crypto from 'crypto';
import type { Libp2p }         from 'libp2p';
import { multiaddr }           from '@multiformats/multiaddr';
import { peerIdFromString }    from '@libp2p/peer-id';
import type { IAgentClient, CallOptions }  from '../../interfaces/IAgentClient';
import type { AgentResponse }             from '../../interfaces/IAgentResponse';
import type { DiscoveryEntry, IAgentDiscovery } from '../../interfaces/IAgentDiscovery';
import type {
  HeartbeatResponse,
  CapabilityExchangeResponse,
  GossipMessage,
  AgentSession,
} from '../../interfaces/IAgentMesh';
import { AgentClient }         from '../../interfaces/IAgentClient';
import { INVOKE_PROTO, readLPFrame, writeLPFrame } from './Libp2PListener';

export interface Libp2PAgentClientOptions {
  callerId?:   string;
  timeoutMs?:  number;
  /** Fallback HTTP client for agents without a peerId in their DiscoveryEntry */
  httpFallback?: IAgentClient;
}

export class Libp2PAgentClient implements IAgentClient {
  private readonly node:        Libp2p;
  private readonly discovery:   IAgentDiscovery;
  private readonly callerId:    string;
  private readonly timeoutMs:   number;
  private readonly http:        IAgentClient;

  constructor(
    node:      Libp2p,
    discovery: IAgentDiscovery,
    opts:      Libp2PAgentClientOptions = {},
  ) {
    this.node       = node;
    this.discovery  = discovery;
    this.callerId   = opts.callerId  ?? 'anonymous';
    this.timeoutMs  = opts.timeoutMs ?? 30_000;
    this.http       = opts.httpFallback ?? new AgentClient(discovery, { callerId: this.callerId, timeoutMs: this.timeoutMs });
  }

  // ── lookup (delegates to discovery) ─────────────────────────────────────────

  async find(capability: string): Promise<DiscoveryEntry | null> {
    const entries = await this.discovery.query(capability);
    const healthy = entries.filter(e => e.health.status === 'healthy');
    return healthy[0] ?? entries[0] ?? null;
  }

  async findAll(capability: string): Promise<DiscoveryEntry[]> {
    const entries = await this.discovery.query(capability);
    const healthy = entries.filter(e => e.health.status === 'healthy');
    return healthy.length > 0 ? healthy : entries;
  }

  async findById(agentId: string): Promise<DiscoveryEntry | null> {
    const all = await this.discovery.listAll();
    return all.find(e => e.agentId === agentId) ?? null;
  }

  // ── interaction ──────────────────────────────────────────────────────────────

  async call(
    agentId:    string,
    capability: string,
    payload:    Record<string, unknown>,
    options:    CallOptions = {},
  ): Promise<AgentResponse> {
    const entry = await this.findById(agentId);
    if (!entry) return errResp(`Agent not found: ${agentId}`);
    return this.callEntry(entry, capability, payload, options);
  }

  async callCapability(
    capability: string,
    payload:    Record<string, unknown>,
    options:    CallOptions = {},
  ): Promise<AgentResponse> {
    const entry = await this.find(capability);
    if (!entry) return errResp(`No agent found for capability: ${capability}`);
    return this.callEntry(entry, capability, payload, options);
  }

  async callEntry(
    entry:      DiscoveryEntry,
    capability: string,
    payload:    Record<string, unknown>,
    options:    CallOptions = {},
  ): Promise<AgentResponse> {
    // Route to libp2p if the entry advertises a peerId; else HTTP fallback
    if (entry.network.peerId) {
      return this._dispatchP2P(entry, capability, payload, options.timeoutMs ?? this.timeoutMs);
    }
    return this.http.callEntry(entry, capability, payload, options);
  }

  // ── mesh ─────────────────────────────────────────────────────────────────────

  async ping(agentId: string, opts: { timeoutMs?: number } = {}): Promise<HeartbeatResponse> {
    return this.http.ping(agentId, opts);
  }

  async connect(entry: DiscoveryEntry, opts: { timeoutMs?: number } = {}): Promise<AgentSession> {
    // Ensure peer is in the node's address book before connecting
    if (entry.network.peerId && entry.network.multiaddr) {
      try {
        const pid  = peerIdFromString(entry.network.peerId);
        const maddr = multiaddr(entry.network.multiaddr);
        await this.node.peerStore.patch(pid, { multiaddrs: [maddr] });
      } catch { /* non-fatal */ }
    }
    return this.http.connect(entry, opts);
  }

  async gossipAnnounce(entry: DiscoveryEntry, opts: { ttl?: number } = {}): Promise<void> {
    return this.http.gossipAnnounce(entry, opts);
  }

  async gossipQuery(capability: string, opts: { ttl?: number; timeoutMs?: number } = {}): Promise<DiscoveryEntry[]> {
    return this.http.gossipQuery(capability, opts);
  }

  // ── libp2p P2P dispatch ───────────────────────────────────────────────────────

  private async _dispatchP2P(
    entry:      DiscoveryEntry,
    capability: string,
    payload:    Record<string, unknown>,
    timeoutMs:  number,
  ): Promise<AgentResponse> {
    const req = {
      requestId:  crypto.randomUUID(),
      from:       this.callerId,
      capability,
      payload,
      timestamp:  Date.now(),
    };

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);

    try {
      const pid    = peerIdFromString(entry.network.peerId!);
      const stream = await this.node.dialProtocol(pid, INVOKE_PROTO, { signal: controller.signal });
      await writeLPFrame(stream, Buffer.from(JSON.stringify(req), 'utf8'));
      const raw  = await readLPFrame(stream);
      await stream.close();
      return JSON.parse(raw.toString('utf8')) as AgentResponse;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return errResp(`libp2p dispatch failed: ${msg}`);
    } finally {
      clearTimeout(timer);
    }
  }
}

function errResp(message: string): AgentResponse {
  return {
    requestId:    crypto.randomUUID(),
    status:       'error',
    errorMessage: message,
    timestamp:    Date.now(),
  } as AgentResponse;
}
