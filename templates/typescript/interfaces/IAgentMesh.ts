/**
 * Borgkit Mesh Protocol — Heartbeat, Capability Exchange, Gossip, and Streaming
 * ─────────────────────────────────────────────────────────────────────────────
 * Defines message types and interfaces for the four built-in agent-to-agent
 * protocols that every Borgkit agent understands:
 *
 *   1. Heartbeat          — liveness ping with status payload
 *   2. Capability Exchange — direct capability query (bypasses discovery layer)
 *   3. Gossip             — capability announcements fan-out across the mesh
 *   4. Streaming          — incremental token / result delivery (SSE or libp2p)
 *
 * Reserved capability names (intercepted before normal dispatch):
 *   "__heartbeat"    → HeartbeatRequest / HeartbeatResponse
 *   "__capabilities" → CapabilityExchangeRequest / CapabilityExchangeResponse
 *   "__gossip"       → GossipMessage (fire-and-forget)
 *
 * Streaming uses POST /invoke/stream and emits Server-Sent Events with
 * StreamChunk frames, terminated by a single StreamEnd frame.
 */

import type { DiscoveryEntry } from './IAgentDiscovery';

// ── Heartbeat ─────────────────────────────────────────────────────────────────

export interface HeartbeatRequest {
  senderId:  string;
  timestamp: number;   // Unix ms
  nonce?:    string;
}

export interface HeartbeatResponse {
  agentId:           string;
  status:            'healthy' | 'degraded' | 'unhealthy';
  timestamp:         number;
  capabilitiesCount: number;
  uptimeMs?:         number;
  version?:          string;
  nonce?:            string;
}

// ── Capability Exchange ───────────────────────────────────────────────────────

export interface CapabilityExchangeRequest {
  senderId:   string;
  timestamp:  number;
  includeAnr: boolean;  // if true, response includes full DiscoveryEntry
}

export interface CapabilityExchangeResponse {
  agentId:      string;
  capabilities: string[];
  timestamp:    number;
  anr?:         DiscoveryEntry;  // full ANR record when includeAnr=true
}

// ── Gossip ────────────────────────────────────────────────────────────────────

export type GossipMessageType = 'announce' | 'revoke' | 'heartbeat' | 'query';

export interface GossipMessage {
  type:        GossipMessageType;
  senderId:    string;
  timestamp:   number;
  ttl:         number;         // decremented each hop; dropped when 0
  seenBy:      string[];       // agent IDs that have forwarded this message
  entry?:      DiscoveryEntry; // present for announce/revoke
  capability?: string;         // present for query
  nonce?:      string;
}

export function forwardGossip(msg: GossipMessage, forwarderId: string): GossipMessage {
  return {
    ...msg,
    ttl:    msg.ttl - 1,
    seenBy: [...msg.seenBy, forwarderId],
  };
}

// ── IGossipProtocol ───────────────────────────────────────────────────────────

export type GossipHandler = (message: GossipMessage) => Promise<void>;

export interface IGossipProtocol {
  /**
   * Fan out a gossip message to all currently connected peers.
   */
  broadcast(message: GossipMessage): Promise<void>;

  /**
   * Process an incoming gossip message from a peer.
   * Implementations should deduplicate, apply to local state, and re-forward.
   */
  receive(message: GossipMessage): Promise<void>;

  /** Register a callback invoked for every incoming gossip message. */
  subscribe(handler: GossipHandler): void;

  /** Return agent IDs of currently connected peers. */
  peers(): string[];

  /** Connect to a new peer. */
  addPeer(agentId: string, endpoint: string): Promise<void>;

  /** Disconnect from a peer. */
  removePeer(agentId: string): Promise<void>;
}

// ── Handshake ─────────────────────────────────────────────────────────────────

/**
 * Result of the connection handshake performed by AgentClient.connect().
 *
 * Capability exchange is part of the handshake — it verifies that the
 * discovered agent still advertises the capabilities you need before the
 * first call is committed.
 */
export interface HandshakeResult {
  agentId:      string;
  healthStatus: 'healthy' | 'degraded' | 'unhealthy';
  capabilities: string[];
  latencyMs:    number;
  connectedAt:  number;   // Unix ms
  anr?:         DiscoveryEntry;
  version?:     string;
}

export function handshakeSupports(h: HandshakeResult, capability: string): boolean {
  return h.capabilities.includes(capability);
}

// ── AgentSession ──────────────────────────────────────────────────────────────

/**
 * An active connection to a remote agent, established by AgentClient.connect().
 *
 * Holds the handshake result (capabilities + health snapshot) and provides
 * call / ping / refreshCapabilities methods that reuse the discovered endpoint
 * without re-querying the discovery layer on every request.
 *
 * @example
 * const session = await client.connect(entry);
 * if (!handshakeSupports(session.handshake, 'weather_forecast')) {
 *   throw new Error('Agent no longer supports weather_forecast');
 * }
 * const resp = await session.call('weather_forecast', { city: 'NYC' });
 */
export interface AgentSession {
  readonly entry:     DiscoveryEntry;
  readonly handshake: HandshakeResult;

  get agentId():     string;
  get capabilities(): string[];
  get isHealthy():   boolean;

  /** Call a capability on this agent using the established session. */
  call(
    capability: string,
    payload:    Record<string, unknown>,
    options?:   { timeoutMs?: number },
  ): Promise<import('./IAgentResponse').AgentResponse>;

  /** Re-check liveness of this agent. */
  ping(options?: { timeoutMs?: number }): Promise<HeartbeatResponse>;

  /**
   * Re-run the capability exchange for this agent.
   * Use after a period of inactivity to verify cached capabilities are still valid.
   */
  refreshCapabilities(options?: { timeoutMs?: number }): Promise<CapabilityExchangeResponse>;

  /**
   * Stream a capability call on this agent using the established session.
   *
   * Yields StreamChunk events as they arrive, followed by a final StreamEnd.
   * Uses the ``POST /invoke/stream`` SSE endpoint on the remote agent.
   *
   * @example
   * ```ts
   * for await (const event of session.stream('summarise', { text: longText })) {
   *   if (event.type === 'chunk') process.stdout.write(event.delta);
   *   else break;  // StreamEnd
   * }
   * ```
   */
  stream(
    capability: string,
    payload:    Record<string, unknown>,
  ): AsyncIterable<StreamChunk | StreamEnd>;

  /** Signal to the remote agent that this session is ending (best-effort). */
  close(): Promise<void>;
}

// ── Streaming ─────────────────────────────────────────────────────────────────

/**
 * A single incremental chunk delivered during a streaming capability call.
 *
 * Sent as an SSE frame on POST /invoke/stream while the agent is still
 * producing output. For LLM agents, `delta` carries the token text.
 * For search or structured agents, `result` carries a partial structured
 * result instead (or in addition).
 *
 * Wire format (SSE):
 *   data: {"type":"chunk","requestId":"…","delta":"…","sequence":N,"timestamp":T}
 */
export interface StreamChunk {
  requestId: string;
  type:      'chunk';
  /** LLM token text or any incremental text delta. */
  delta:     string;
  /** Structured partial result (optional, for non-text agents). */
  result?:   unknown;
  /** Monotonically increasing counter per request. */
  sequence:  number;
  /** Unix timestamp (ms). */
  timestamp: number;
}

/**
 * Terminal frame of a streaming capability call.
 *
 * Sent as the last SSE frame on POST /invoke/stream. `finalResult` carries
 * the complete assembled result. `error` is set on abnormal termination.
 *
 * Wire format (SSE):
 *   data: {"type":"end","requestId":"…","finalResult":{…},"sequence":N,"timestamp":T}
 */
export interface StreamEnd {
  requestId:    string;
  type:         'end';
  /** Complete assembled result (for callers that only want the final value). */
  finalResult?: unknown;
  /** Non-null on error or cancellation. */
  error?:       string;
  sequence:     number;
  timestamp:    number;
}
