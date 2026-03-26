/**
 * GossipDiscovery — capability propagation via peer-to-peer gossip fan-out.
 *
 * Extends in-memory discovery with gossip: on register/unregister, fans out
 * announce/revoke GossipMessages to all known peers. Incoming gossip updates
 * the local registry and is re-forwarded with ttl-1.
 *
 * @example
 * const registry = new GossipDiscovery('sentrix://agent/me');
 * await registry.addPeer('sentrix://agent/peer-a', 'http://peer-a:6174');
 * await registry.register(myEntry);  // auto-gossips to peers
 */

import type { DiscoveryEntry }             from '../interfaces/IAgentDiscovery';
import type { GossipHandler, IGossipProtocol } from '../interfaces/IAgentMesh';
import {
  GossipMessage,
  forwardGossip,
}                                          from '../interfaces/IAgentMesh';

export class GossipDiscovery implements IGossipProtocol {
  private readonly registry  = new Map<string, DiscoveryEntry>();
  private readonly peers_map = new Map<string, string>();  // agentId → endpoint
  private readonly handlers: GossipHandler[] = [];
  private readonly seen = new Set<string>();
  private readonly agentId: string;
  private readonly defaultTtl: number;

  constructor(agentId: string, defaultTtl = 3) {
    this.agentId    = agentId;
    this.defaultTtl = defaultTtl;
  }

  // ── IAgentDiscovery ───────────────────────────────────────────────────────

  async register(entry: DiscoveryEntry): Promise<void> {
    this.registry.set(entry.agentId, entry);
    await this.broadcast({
      type:      'announce',
      senderId:  this.agentId,
      timestamp: Date.now(),
      ttl:       this.defaultTtl,
      seenBy:    [],
      entry,
    });
  }

  async unregister(agentId: string): Promise<void> {
    const entry = this.registry.get(agentId);
    this.registry.delete(agentId);
    if (entry) {
      await this.broadcast({
        type:      'revoke',
        senderId:  this.agentId,
        timestamp: Date.now(),
        ttl:       this.defaultTtl,
        seenBy:    [],
        entry,
      });
    }
  }

  async query(capability: string): Promise<DiscoveryEntry[]> {
    return [...this.registry.values()].filter(
      e => e.capabilities.includes(capability) && e.health.status !== 'unhealthy',
    );
  }

  async listAll(): Promise<DiscoveryEntry[]> {
    return [...this.registry.values()];
  }

  async heartbeat(agentId: string): Promise<void> {
    const entry = this.registry.get(agentId);
    if (entry) {
      entry.health = { status: 'healthy', lastHeartbeat: new Date().toISOString(), uptimeSeconds: 0 };
    }
    await this.broadcast({
      type:      'heartbeat',
      senderId:  agentId,
      timestamp: Date.now(),
      ttl:       1,
      seenBy:    [],
    });
  }

  // ── IGossipProtocol ───────────────────────────────────────────────────────

  async broadcast(message: GossipMessage): Promise<void> {
    const tasks = [...this.peers_map.entries()]
      .filter(([id]) => !message.seenBy.includes(id))
      .map(([, endpoint]) => this.sendGossip(endpoint, message));
    await Promise.allSettled(tasks);
  }

  async receive(message: GossipMessage): Promise<void> {
    const key = `${message.senderId}:${message.timestamp}:${message.nonce ?? ''}`;
    if (this.seen.has(key)) return;
    this.seen.add(key);
    if (this.seen.size > 10_000) {
      const toDelete = [...this.seen].slice(0, 5_000);
      toDelete.forEach(k => this.seen.delete(k));
    }

    if (message.type === 'announce' && message.entry) {
      this.registry.set(message.entry.agentId, message.entry);
    } else if (message.type === 'revoke' && message.entry) {
      this.registry.delete(message.entry.agentId);
    } else if (message.type === 'heartbeat') {
      const e = this.registry.get(message.senderId);
      if (e) e.health = { status: 'healthy', lastHeartbeat: new Date().toISOString(), uptimeSeconds: 0 };
    }

    for (const handler of this.handlers) {
      try { await handler(message); } catch { /* best-effort */ }
    }

    if (message.ttl > 0) {
      await this.broadcast(forwardGossip(message, this.agentId));
    }
  }

  subscribe(handler: GossipHandler): void {
    this.handlers.push(handler);
  }

  peers(): string[] {
    return [...this.peers_map.keys()];
  }

  async addPeer(agentId: string, endpoint: string): Promise<void> {
    this.peers_map.set(agentId, endpoint);
  }

  async removePeer(agentId: string): Promise<void> {
    this.peers_map.delete(agentId);
  }

  // ── internal ──────────────────────────────────────────────────────────────

  private async sendGossip(endpoint: string, message: GossipMessage): Promise<void> {
    const url = endpoint.replace(/\/$/, '') + '/gossip';
    try {
      await fetch(url, {
        method:  'POST',
        headers: { 'Content-Type': 'application/json' },
        body:    JSON.stringify(message),
        signal:  AbortSignal.timeout(3_000),
      });
    } catch { /* fire-and-forget */ }
  }
}
