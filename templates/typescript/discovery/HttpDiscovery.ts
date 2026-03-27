/**
 * HttpDiscovery — centralised discovery adapter (optional extension).
 *
 * Connects to any REST-based agent registry that implements the Borgkit
 * centralised discovery API. This is NOT the default; LocalDiscovery and
 * GossipDiscovery are preferred. Use this as an escape hatch when:
 *   - bootstrapping a new network
 *   - interoperating with a managed registry
 *   - operating in an enterprise / firewalled environment
 *
 * The server must expose:
 *   POST   /agents          → register
 *   DELETE /agents/:id      → unregister
 *   GET    /agents?cap=X    → query by capability
 *   GET    /agents          → list all
 *   PUT    /agents/:id/hb   → heartbeat
 */

import { IAgentDiscovery, DiscoveryEntry } from '../interfaces/IAgentDiscovery';

export interface HttpDiscoveryOptions {
  /** Base URL of the centralised registry, e.g. "https://registry.borgkit.io" */
  baseUrl: string;
  /** Optional API key for authenticated registries */
  apiKey?: string;
  /** Request timeout in milliseconds (default: 5000) */
  timeoutMs?: number;
  /** Heartbeat interval in milliseconds. 0 = disabled (default: 30_000) */
  heartbeatIntervalMs?: number;
}

export class HttpDiscovery implements IAgentDiscovery {
  private readonly base: string;
  private readonly headers: Record<string, string>;
  private readonly timeoutMs: number;
  private readonly heartbeatMs: number;
  private heartbeatTimers = new Map<string, ReturnType<typeof setInterval>>();

  constructor(opts: HttpDiscoveryOptions) {
    this.base        = opts.baseUrl.replace(/\/$/, '');
    this.timeoutMs   = opts.timeoutMs          ?? 5_000;
    this.heartbeatMs = opts.heartbeatIntervalMs ?? 30_000;
    this.headers     = {
      'Content-Type': 'application/json',
      ...(opts.apiKey ? { 'X-Api-Key': opts.apiKey } : {}),
    };
  }

  async register(entry: DiscoveryEntry): Promise<void> {
    await this.request('POST', '/agents', entry);

    // Start background heartbeat if enabled
    if (this.heartbeatMs > 0) {
      const timer = setInterval(
        () => this.heartbeat(entry.agentId).catch(console.warn),
        this.heartbeatMs
      );
      this.heartbeatTimers.set(entry.agentId, timer);
    }
    console.log(`[HttpDiscovery] Registered: ${entry.agentId} → ${this.base}`);
  }

  async unregister(agentId: string): Promise<void> {
    // Stop heartbeat
    const timer = this.heartbeatTimers.get(agentId);
    if (timer) { clearInterval(timer); this.heartbeatTimers.delete(agentId); }
    await this.request('DELETE', `/agents/${encodeURIComponent(agentId)}`);
  }

  async query(capability: string): Promise<DiscoveryEntry[]> {
    return this.request<DiscoveryEntry[]>(
      'GET', `/agents?cap=${encodeURIComponent(capability)}`
    );
  }

  async listAll(): Promise<DiscoveryEntry[]> {
    return this.request<DiscoveryEntry[]>('GET', '/agents');
  }

  async heartbeat(agentId: string): Promise<void> {
    await this.request('PUT', `/agents/${encodeURIComponent(agentId)}/hb`);
  }

  // ── internals ──────────────────────────────────────────────────────────────

  private async request<T = void>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<T> {
    const controller = new AbortController();
    const tid = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const res = await fetch(`${this.base}${path}`, {
        method,
        headers: this.headers,
        body:    body ? JSON.stringify(body) : undefined,
        signal:  controller.signal,
      });

      if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`[HttpDiscovery] ${method} ${path} → ${res.status}: ${text}`);
      }

      const ct = res.headers.get('content-type') ?? '';
      if (ct.includes('application/json') && res.status !== 204) {
        return res.json() as Promise<T>;
      }
      return undefined as T;
    } finally {
      clearTimeout(tid);
    }
  }
}
