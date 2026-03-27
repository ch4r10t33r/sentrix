/**
 * IAgentClient — standard interface for discovering and calling Borgkit agents.
 *
 * Combines lookup (find by capability / agent ID) with invocation
 * (send AgentRequest, receive AgentResponse) in a single coherent API.
 *
 * Quick start
 * -----------
 * import { AgentClient }     from './IAgentClient';
 * import { DiscoveryFactory } from '../discovery/DiscoveryFactory';
 *
 * const discovery = DiscoveryFactory.create({ type: 'local' });
 * const client    = new AgentClient(discovery);
 *
 * // Discover-and-call in one step:
 * const resp = await client.callCapability('weather_forecast', { city: 'NYC' });
 *
 * // With x402 auto-payment:
 * import { MockWalletProvider } from '../addons/x402/client';
 * const client = new AgentClient(discovery, { x402Wallet: new MockWalletProvider(), autoPay: true });
 * const resp = await client.callCapability('premium_analysis', { query: '...' });
 */

import type { AgentResponse }  from './IAgentResponse';
import type { DiscoveryEntry } from './IAgentDiscovery';
import type { IAgentDiscovery } from './IAgentDiscovery';
import { AgentRequest }        from './IAgentRequest';  // will use for construction
import * as crypto             from 'crypto';
import type {
  HeartbeatRequest,
  HeartbeatResponse,
  CapabilityExchangeRequest,
  CapabilityExchangeResponse,
  GossipMessage,
  HandshakeResult,
  AgentSession,
  StreamChunk,
  StreamEnd,
} from './IAgentMesh';

// ── interface ─────────────────────────────────────────────────────────────────

export interface IAgentClient {

  // ── Lookup ─────────────────────────────────────────────────────────────────

  /**
   * Find the best healthy agent that exposes `capability`.
   * Returns null if no agent is registered for this capability.
   */
  find(capability: string): Promise<DiscoveryEntry | null>;

  /** Return all healthy agents that expose `capability`. */
  findAll(capability: string): Promise<DiscoveryEntry[]>;

  /**
   * Look up a specific agent by agent ID.
   * Returns null if not found in the discovery layer.
   */
  findById(agentId: string): Promise<DiscoveryEntry | null>;

  // ── Interaction ────────────────────────────────────────────────────────────

  /**
   * Call a specific agent by its agentId.
   *
   * Looks up the agent's endpoint via discovery, builds an AgentRequest,
   * and dispatches it over HTTP transport to `{protocol}://{host}:{port}/invoke`.
   */
  call(
    agentId:    string,
    capability: string,
    payload:    Record<string, unknown>,
    options?:   CallOptions,
  ): Promise<AgentResponse>;

  /**
   * Discover the best agent for `capability` then call it in one step.
   * Returns an error AgentResponse if no healthy agent is found.
   */
  callCapability(
    capability: string,
    payload:    Record<string, unknown>,
    options?:   CallOptions,
  ): Promise<AgentResponse>;

  /**
   * Call an agent using a DiscoveryEntry you already have (skips lookup).
   */
  callEntry(
    entry:      DiscoveryEntry,
    capability: string,
    payload:    Record<string, unknown>,
    options?:   CallOptions,
  ): Promise<AgentResponse>;

  // ── Mesh protocols ─────────────────────────────────────────────────────────

  /**
   * Send a heartbeat ping to an agent and return its response.
   * Returns status='unhealthy' if the agent is unreachable.
   */
  ping(agentId: string, options?: { timeoutMs?: number }): Promise<HeartbeatResponse>;

  /**
   * Establish a connection to a remote agent via a two-step handshake:
   *
   *   1. Heartbeat ping  — confirms liveness and health status.
   *   2. Capability exchange — verifies the agent's current capabilities match
   *      what was advertised in the discovery layer before the first call.
   *
   * Returns an AgentSession that caches the handshake result and provides
   * call / ping / refreshCapabilities methods for subsequent interactions.
   *
   * @example
   * const entry   = await client.find('weather_forecast');
   * const session = await client.connect(entry!);
   * if (handshakeSupports(session.handshake, 'weather_forecast')) {
   *   const resp = await session.call('weather_forecast', { city: 'NYC' });
   * }
   */
  connect(
    entry:    DiscoveryEntry,
    options?: { timeoutMs?: number },
  ): Promise<AgentSession>;

  /**
   * Broadcast a capability announcement to all connected peers.
   */
  gossipAnnounce(entry: DiscoveryEntry, options?: { ttl?: number }): Promise<void>;

  /**
   * Broadcast a capability query across the gossip mesh and collect responses.
   */
  gossipQuery(
    capability: string,
    options?:   { ttl?: number; timeoutMs?: number },
  ): Promise<DiscoveryEntry[]>;

  // ── Streaming ──────────────────────────────────────────────────────────────

  /**
   * Stream a capability call to a specific agent by agentId.
   *
   * Connects to `POST {agent_endpoint}/invoke/stream` and yields
   * `StreamChunk` objects as the remote agent produces output, followed by a
   * single `StreamEnd` frame.
   *
   * @example
   * ```ts
   * for await (const event of client.stream('borgkit://agent/llm', 'generate', { prompt })) {
   *   if (event.type === 'chunk') process.stdout.write(event.delta);
   *   else break;
   * }
   * ```
   */
  stream(
    agentId:    string,
    capability: string,
    payload:    Record<string, unknown>,
    options?:   StreamOptions,
  ): AsyncIterable<StreamChunk | StreamEnd>;

  /**
   * Discover the best agent for `capability` then stream it in one step.
   */
  streamCapability(
    capability: string,
    payload:    Record<string, unknown>,
    options?:   StreamOptions,
  ): AsyncIterable<StreamChunk | StreamEnd>;

  /**
   * Stream a capability call using a DiscoveryEntry you already have.
   * Skips the lookup step.
   */
  streamEntry(
    entry:      DiscoveryEntry,
    capability: string,
    payload:    Record<string, unknown>,
    options?:   StreamOptions,
  ): AsyncIterable<StreamChunk | StreamEnd>;
}

export interface StreamOptions {
  /** Identity of the calling agent (default: "anonymous"). */
  callerId?: string;
}

export interface CallOptions {
  /** Identity of the calling agent (default: "anonymous"). */
  callerId?:   string;
  /** Request timeout in milliseconds (default: 30 000). */
  timeoutMs?:  number;
}

// ── AgentClient — HTTP transport implementation ───────────────────────────────

export interface AgentClientOptions {
  /** Default caller identity injected into every AgentRequest. */
  callerId?:   string;
  /** Default HTTP timeout in milliseconds. */
  timeoutMs?:  number;
  /**
   * WalletProvider for automatic x402 payment handling.
   * If omitted, payment_required responses are returned as-is.
   */
  x402Wallet?: import('../addons/x402/client').WalletProvider;
  /** If true, pays x402 challenges without calling onPaymentRequired(). */
  autoPay?:    boolean;
}

export class AgentClient implements IAgentClient {
  private readonly discovery:  IAgentDiscovery;
  private readonly callerId:   string;
  private readonly timeoutMs:  number;
  private readonly x402Wallet: import('../addons/x402/client').WalletProvider | undefined;
  private readonly autoPay:    boolean;

  constructor(discovery: IAgentDiscovery, options: AgentClientOptions = {}) {
    this.discovery  = discovery;
    this.callerId   = options.callerId  ?? 'anonymous';
    this.timeoutMs  = options.timeoutMs ?? 30_000;
    this.x402Wallet = options.x402Wallet;
    this.autoPay    = options.autoPay ?? false;
  }

  // ── lookup ──────────────────────────────────────────────────────────────────

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

  // ── interaction ─────────────────────────────────────────────────────────────

  async call(
    agentId: string,
    capability: string,
    payload: Record<string, unknown>,
    options: CallOptions = {},
  ): Promise<AgentResponse> {
    const entry = await this.findById(agentId);
    if (!entry) {
      return errorResponse(`Agent not found in discovery: ${agentId}`);
    }
    return this.callEntry(entry, capability, payload, options);
  }

  async callCapability(
    capability: string,
    payload: Record<string, unknown>,
    options: CallOptions = {},
  ): Promise<AgentResponse> {
    const entry = await this.find(capability);
    if (!entry) {
      return errorResponse(`No healthy agent found for capability: '${capability}'`);
    }
    return this.callEntry(entry, capability, payload, options);
  }

  async callEntry(
    entry: DiscoveryEntry,
    capability: string,
    payload: Record<string, unknown>,
    options: CallOptions = {},
  ): Promise<AgentResponse> {
    const req = buildRequest(
      options.callerId ?? this.callerId,
      capability,
      payload,
    );
    return this.dispatch(entry, req, options.timeoutMs ?? this.timeoutMs);
  }

  async ping(
    agentId: string,
    options: { timeoutMs?: number } = {},
  ): Promise<HeartbeatResponse> {
    const req: HeartbeatRequest = { senderId: this.callerId, timestamp: Date.now() };
    const resp = await this.call(agentId, '__heartbeat', req as any, { timeoutMs: options.timeoutMs ?? 5_000 });
    if (resp.status === 'success' && resp.result) {
      return resp.result as unknown as HeartbeatResponse;
    }
    return { agentId, status: 'unhealthy', timestamp: Date.now(), capabilitiesCount: 0 };
  }

  async connect(
    entry:   DiscoveryEntry,
    options: { timeoutMs?: number } = {},
  ): Promise<AgentSession> {
    const timeoutMs = options.timeoutMs ?? this.timeoutMs;
    const t0 = Date.now();

    // Step 1: heartbeat (liveness + health)
    const hb = await this.ping(entry.agentId, { timeoutMs });

    // Step 2: capability exchange (verify current capabilities)
    const capResp = await this._exchangeCapabilities(entry, timeoutMs);

    const handshake: HandshakeResult = {
      agentId:      entry.agentId,
      healthStatus: hb.status,
      capabilities: capResp.capabilities,
      latencyMs:    Date.now() - t0,
      connectedAt:  Date.now(),
      anr:          capResp.anr,
      version:      hb.version,
    };
    return new AgentSessionImpl(entry, handshake, this);
  }

  /** Internal: capability exchange against a known endpoint. */
  async _exchangeCapabilities(
    entry:     DiscoveryEntry,
    timeoutMs: number,
  ): Promise<CapabilityExchangeResponse> {
    const req: CapabilityExchangeRequest = {
      senderId:   this.callerId,
      timestamp:  Date.now(),
      includeAnr: true,
    };
    const resp = await this.callEntry(entry, '__capabilities', req as any, { timeoutMs });
    if (resp.status === 'success' && resp.result) {
      return resp.result as unknown as CapabilityExchangeResponse;
    }
    return { agentId: entry.agentId, capabilities: [], timestamp: Date.now() };
  }

  async gossipAnnounce(entry: DiscoveryEntry, options: { ttl?: number } = {}): Promise<void> {
    const msg: GossipMessage = {
      type:      'announce',
      senderId:  this.callerId,
      timestamp: Date.now(),
      ttl:       options.ttl ?? 3,
      seenBy:    [],
      entry,
    };
    const peers = await this.discovery.listAll();
    await Promise.allSettled(
      peers
        .filter(p => p.agentId !== this.callerId)
        .map(p => this.callEntry(p, '__gossip', msg as any, { timeoutMs: 2_000 })),
    );
  }

  async gossipQuery(
    capability: string,
    options: { ttl?: number; timeoutMs?: number } = {},
  ): Promise<DiscoveryEntry[]> {
    const msg: GossipMessage = {
      type:       'query',
      senderId:   this.callerId,
      timestamp:  Date.now(),
      ttl:        options.ttl ?? 3,
      seenBy:     [],
      capability,
    };
    const peers = await this.discovery.listAll();
    const results: DiscoveryEntry[] = [];
    await Promise.allSettled(
      peers
        .filter(p => p.agentId !== this.callerId)
        .map(async p => {
          const resp = await this.callEntry(p, '__gossip', msg as any, { timeoutMs: options.timeoutMs ?? 5_000 });
          if (resp.status === 'success' && (resp.result as any)?.entries) {
            results.push(...(resp.result as any).entries as DiscoveryEntry[]);
          }
        }),
    );
    return results;
  }

  // ── streaming ────────────────────────────────────────────────────────────────

  async *stream(
    agentId:    string,
    capability: string,
    payload:    Record<string, unknown>,
    options:    StreamOptions = {},
  ): AsyncIterable<StreamChunk | StreamEnd> {
    const entry = await this.findById(agentId);
    if (!entry) {
      yield { requestId: crypto.randomUUID(), type: 'end', error: `Agent not found in discovery: ${agentId}`, sequence: 0, timestamp: Date.now() };
      return;
    }
    yield* this.streamEntry(entry, capability, payload, options);
  }

  async *streamCapability(
    capability: string,
    payload:    Record<string, unknown>,
    options:    StreamOptions = {},
  ): AsyncIterable<StreamChunk | StreamEnd> {
    const entry = await this.find(capability);
    if (!entry) {
      yield { requestId: crypto.randomUUID(), type: 'end', error: `No healthy agent found for capability: '${capability}'`, sequence: 0, timestamp: Date.now() };
      return;
    }
    yield* this.streamEntry(entry, capability, payload, options);
  }

  async *streamEntry(
    entry:      DiscoveryEntry,
    capability: string,
    payload:    Record<string, unknown>,
    options:    StreamOptions = {},
  ): AsyncIterable<StreamChunk | StreamEnd> {
    const requestId = crypto.randomUUID();
    const callerId  = options.callerId ?? this.callerId;
    const url       = streamEndpointUrl(entry);
    const body      = JSON.stringify({
      requestId,
      from:       callerId,
      capability,
      payload,
      timestamp:  Date.now(),
      stream:     true,
    });

    try {
      yield* httpStream(url, body);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      yield { requestId, type: 'end', error: msg, sequence: 0, timestamp: Date.now() };
    }
  }

  // ── transport ────────────────────────────────────────────────────────────────

  private async dispatch(
    entry: DiscoveryEntry,
    req: Record<string, unknown>,
    timeoutMs: number,
  ): Promise<AgentResponse> {
    const url = endpointUrl(entry);
    let resp = await httpPost(url, req, timeoutMs);

    // x402 auto-payment
    if (resp.status === 'payment_required' && this.x402Wallet) {
      const reqs: unknown[] = (resp as any).paymentRequirements ?? [];
      if (reqs.length > 0) {
        const { X402PaymentRequirements } = await import('../addons/x402/types');
        const requirements = X402PaymentRequirements.fromDict(reqs[0] as Record<string, unknown>);
        const payment = await this.x402Wallet.signPayment(requirements, req as any);
        const paidReq = { ...req, x402: payment };
        resp = await httpPost(url, paidReq, timeoutMs);
      }
    }

    return resp;
  }
}

// ── AgentSessionImpl ──────────────────────────────────────────────────────────

class AgentSessionImpl implements AgentSession {
  readonly entry:     DiscoveryEntry;
  readonly handshake: HandshakeResult;
  private  readonly client: AgentClient;

  constructor(entry: DiscoveryEntry, handshake: HandshakeResult, client: AgentClient) {
    this.entry     = entry;
    this.handshake = handshake;
    this.client    = client;
  }

  get agentId():     string  { return this.handshake.agentId; }
  get capabilities(): string[] { return this.handshake.capabilities; }
  get isHealthy():   boolean  { return this.handshake.healthStatus === 'healthy'; }

  async call(
    capability: string,
    payload:    Record<string, unknown>,
    options:    { timeoutMs?: number } = {},
  ): Promise<AgentResponse> {
    return this.client.callEntry(this.entry, capability, payload, options);
  }

  async ping(options: { timeoutMs?: number } = {}): Promise<HeartbeatResponse> {
    return this.client.ping(this.entry.agentId, options);
  }

  async refreshCapabilities(
    options: { timeoutMs?: number } = {},
  ): Promise<CapabilityExchangeResponse> {
    return this.client._exchangeCapabilities(
      this.entry,
      options.timeoutMs ?? 10_000,
    );
  }

  async *stream(
    capability: string,
    payload:    Record<string, unknown>,
  ): AsyncIterable<StreamChunk | StreamEnd> {
    yield* this.client.streamEntry(this.entry, capability, payload);
  }

  async close(): Promise<void> {
    try {
      await this.client.callEntry(
        this.entry,
        '__disconnect',
        { sessionAgentId: this.agentId },
        { timeoutMs: 2_000 },
      );
    } catch { /* best-effort */ }
  }
}

// ── helpers ───────────────────────────────────────────────────────────────────

function endpointUrl(entry: DiscoveryEntry): string {
  const scheme = entry.network.tls
    ? 'https'
    : ['http', 'https'].includes(entry.network.protocol) ? entry.network.protocol : 'http';
  return `${scheme}://${entry.network.host}:${entry.network.port}/invoke`;
}

function streamEndpointUrl(entry: DiscoveryEntry): string {
  const scheme = entry.network.tls
    ? 'https'
    : ['http', 'https'].includes(entry.network.protocol) ? entry.network.protocol : 'http';
  return `${scheme}://${entry.network.host}:${entry.network.port}/invoke/stream`;
}

/**
 * Open a streaming POST request to *url* and yield parsed StreamChunk /
 * StreamEnd objects from the SSE response.
 *
 * Uses the Fetch API with a ReadableStream body reader.  Works in Node 18+
 * (native fetch) and any browser environment.
 */
async function* httpStream(
  url:  string,
  body: string,
): AsyncIterable<StreamChunk | StreamEnd> {
  const res = await fetch(url, {
    method:  'POST',
    headers: {
      'Content-Type': 'application/json',
      'Accept':       'text/event-stream',
    },
    body,
  });

  if (!res.ok || !res.body) {
    // Non-streaming error response — wrap in a StreamEnd
    let errText = `HTTP ${res.status}`;
    try { errText = await res.text(); } catch { /* ignore */ }
    yield { requestId: '', type: 'end', error: errText, sequence: 0, timestamp: Date.now() };
    return;
  }

  const reader  = res.body.getReader();
  const decoder = new TextDecoder();
  let   buffer  = '';

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });

    // SSE lines are separated by '\n\n' (double newline)
    const lines = buffer.split('\n');
    buffer = lines.pop() ?? '';   // keep any incomplete line in the buffer

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed.startsWith('data:')) continue;
      const jsonStr = trimmed.slice(5).trim();
      if (!jsonStr) continue;
      let d: Record<string, unknown>;
      try { d = JSON.parse(jsonStr); } catch { continue; }
      const eventType = (d.type as string) ?? 'chunk';
      if (eventType === 'end') {
        yield {
          requestId:   (d.requestId as string) ?? '',
          type:        'end',
          finalResult: d.finalResult,
          error:       d.error as string | undefined,
          sequence:    (d.sequence as number) ?? 0,
          timestamp:   (d.timestamp as number) ?? Date.now(),
        };
        return;
      }
      yield {
        requestId: (d.requestId as string) ?? '',
        type:      'chunk',
        delta:     (d.delta as string) ?? '',
        result:    d.result,
        sequence:  (d.sequence as number) ?? 0,
        timestamp: (d.timestamp as number) ?? Date.now(),
      };
    }
  }
}

function buildRequest(
  callerId: string,
  capability: string,
  payload: Record<string, unknown>,
): Record<string, unknown> {
  return {
    requestId:  crypto.randomUUID(),
    from:       callerId,
    capability,
    payload,
    timestamp:  Date.now(),
  };
}

async function httpPost(
  url: string,
  body: Record<string, unknown>,
  timeoutMs: number,
): Promise<AgentResponse> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(url, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify(body),
      signal:  controller.signal,
    });
    const data = await res.json() as Record<string, unknown>;
    return normaliseResponse(data);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    return errorResponse(`HTTP request failed: ${msg}`);
  } finally {
    clearTimeout(timer);
  }
}

function normaliseResponse(d: Record<string, unknown>): AgentResponse {
  return {
    requestId:          (d.requestId ?? d.request_id ?? '') as string,
    status:             (d.status ?? 'error') as AgentResponse['status'],
    result:             d.result as Record<string, unknown> | undefined,
    errorMessage:       (d.errorMessage ?? d.error_message) as string | undefined,
    proof:              d.proof as string | undefined,
    signature:          d.signature as string | undefined,
    timestamp:          (d.timestamp ?? Date.now()) as number,
    paymentRequirements: d.paymentRequirements as unknown[] | undefined,
  } as AgentResponse;
}

function errorResponse(message: string): AgentResponse {
  return {
    requestId:    crypto.randomUUID(),
    status:       'error',
    errorMessage: message,
    timestamp:    Date.now(),
  } as AgentResponse;
}
