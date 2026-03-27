import { AgentRequest }   from './IAgentRequest';
import { AgentResponse }  from './IAgentResponse';
import { DiscoveryEntry } from './IAgentDiscovery';
import type {
  HeartbeatRequest,
  HeartbeatResponse,
  CapabilityExchangeRequest,
  CapabilityExchangeResponse,
  GossipMessage,
  StreamChunk,
  StreamEnd,
} from './IAgentMesh';

/**
 * Borgkit agent interface.
 * Every Borgkit agent must implement this contract.
 *
 * Identity note
 * ─────────────
 * `agentId` is required. `owner` is optional — a local secp256k1 key is
 * sufficient for signed ANR records and P2P discovery without an on-chain
 * wallet. ERC-8004 on-chain registration remains available as an opt-in.
 * See identity/IdentityProvider for the LocalKeystoreIdentity default.
 */
export interface IAgent {
  // ─── Identity ─────────────────────────────────────────────────────────────
  /** Borgkit agent URI, e.g. "borgkit://agent/0xABC..." */
  readonly agentId: string;
  /**
   * Owner identifier — Ethereum address when using ERC-8004 or key-derived
   * identity; any unique string otherwise. Defaults to "anonymous".
   */
  readonly owner?: string;
  /** Optional IPFS / on-chain metadata URI */
  readonly metadataUri?: string;
  /** Human-readable metadata bag */
  readonly metadata?: AgentMetadata;

  // ─── Capabilities ─────────────────────────────────────────────────────────
  /** Return the list of capability names this agent exposes */
  getCapabilities(): string[];

  // ─── Request handling ─────────────────────────────────────────────────────
  /** Primary dispatch method — all inbound calls arrive here */
  handleRequest(request: AgentRequest): Promise<AgentResponse>;

  /**
   * Streaming variant of handleRequest.
   *
   * Yields `StreamChunk` objects as incremental output is produced, then a
   * single `StreamEnd` to signal completion.
   *
   * The default implementation (provided by WrappedAgent) falls back to
   * `handleRequest` and emits the full result as one chunk + StreamEnd, so all
   * agents support `POST /invoke/stream` without any changes.
   *
   * Override in framework plugins that produce genuine token streams.
   *
   * @example
   * ```ts
   * async *streamRequest(req) {
   *   let seq = 0;
   *   for await (const token of myLlm.stream(req.payload.prompt)) {
   *     yield { requestId: req.requestId, type: 'chunk', delta: token, sequence: seq++, timestamp: Date.now() };
   *   }
   *   yield { requestId: req.requestId, type: 'end', sequence: seq, timestamp: Date.now() };
   * }
   * ```
   */
  streamRequest?(request: AgentRequest): AsyncIterable<StreamChunk | StreamEnd>;

  /** Optional pre-processing hook (auth, rate-limit, logging…) */
  preProcess?(request: AgentRequest): Promise<void>;
  /** Optional post-processing hook (audit log, billing…) */
  postProcess?(response: AgentResponse): Promise<void>;

  // ─── Discovery (optional) ─────────────────────────────────────────────────
  /** Announce this agent to the discovery layer */
  registerDiscovery?(): Promise<void>;
  /** Gracefully withdraw from the discovery layer */
  unregisterDiscovery?(): Promise<void>;

  // ─── Delegation / permissions (optional) ─────────────────────────────────
  /** Return true if `caller` is permitted to invoke `capability` */
  checkPermission?(caller: string, capability: string): Promise<boolean>;

  // ─── Mesh protocols (heartbeat / capability exchange / gossip) ────────────

  /**
   * Respond to a heartbeat ping from another agent.
   * Default: returns status='healthy' with capability count.
   */
  handleHeartbeat?(req: HeartbeatRequest): Promise<HeartbeatResponse>;

  /**
   * Respond to a direct capability query from another agent.
   * Default: returns capabilities list and full ANR.
   */
  handleCapabilityExchange?(req: CapabilityExchangeRequest): Promise<CapabilityExchangeResponse>;

  /**
   * Process an incoming gossip message.
   * Default: no-op. Override to react to announce/revoke/query messages.
   */
  handleGossip?(msg: GossipMessage): Promise<void>;

  // ─── ANR / Identity exposure ──────────────────────────────────────────────
  /**
   * Return the full ANR (Agent Network Record) for this agent.
   *
   * The ANR is the authoritative self-description of the agent on the mesh:
   * its identity, capabilities, network endpoint, and health status.
   * Callers can use this to inspect a live agent without querying the
   * discovery layer.
   */
  getAnr(): DiscoveryEntry;

  /**
   * Return the libp2p PeerId derived from this agent's secp256k1 ANR key.
   *
   * The PeerId is derived from the same key used to sign ANR records —
   * one keypair, one identity across both the ANR layer and the P2P transport.
   *
   * Returns null for anonymous agents (no signing key configured).
   */
  getPeerId(): Promise<string | null>;

  // ─── Signing (optional) ───────────────────────────────────────────────────
  /** EIP-712 compatible message signing */
  signMessage?(message: string): Promise<string>;
}

export interface AgentMetadata {
  name: string;
  version: string;
  description?: string;
  author?: string;
  license?: string;
  repository?: string;
  tags?: string[];
  resourceRequirements?: {
    minMemoryMb?: number;
    minCpuCores?: number;
    storageGb?: number;
  };
}
