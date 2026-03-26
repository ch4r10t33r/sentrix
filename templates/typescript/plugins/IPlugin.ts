/**
 * Sentrix Plugin Interface (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Defines the contract every framework adapter must satisfy.
 * Implement this to bring any TypeScript agent framework into Sentrix.
 */

import { IAgent }         from '../interfaces/IAgent';
import { AgentRequest }   from '../interfaces/IAgentRequest';
import { AgentResponse }  from '../interfaces/IAgentResponse';
import { DiscoveryEntry } from '../interfaces/IAgentDiscovery';

// ── config ────────────────────────────────────────────────────────────────────

export interface PluginConfig {
  // Identity
  agentId:      string;
  owner:        string;
  name:         string;
  version:      string;
  description?: string;
  tags?:        string[];
  metadataUri?: string;

  // Network
  host?:     string;     // default: 'localhost'
  port?:     number;     // default: 6174
  protocol?: string;     // default: 'http'
  tls?:      boolean;

  // Discovery
  discoveryType?: 'local' | 'http' | 'gossip';
  discoveryUrl?:  string;
  discoveryKey?:  string;

  // Optional ANR signing key (32-byte hex)
  signingKey?: string;

  /**
   * Map Sentrix capability names → framework-native tool/function names.
   * e.g. { "getWeather": "weather_tool" }
   */
  capabilityMap?: Record<string, string>;
}

// ── capability descriptor ─────────────────────────────────────────────────────

export interface CapabilityDescriptor {
  /** Sentrix capability name (what callers use) */
  name:          string;
  description:   string;
  inputSchema?:  Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  tags?:         string[];
  /** The native tool/function name inside the framework */
  nativeName:    string;
}

// ── plugin interface ──────────────────────────────────────────────────────────

export interface ISentrixPlugin<TAgent = unknown, TNativeInput = unknown, TNativeOutput = unknown> {
  readonly config: PluginConfig;

  /** Inspect the framework agent and return its capabilities. */
  extractCapabilities(agent: TAgent): CapabilityDescriptor[];

  /** AgentRequest → framework-native invocation input */
  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): TNativeInput;

  /** Framework output → AgentResponse */
  translateResponse(nativeResult: TNativeOutput, requestId: string): AgentResponse;

  /** Execute the framework agent with the translated input. */
  invokeNative(
    agent:       TAgent,
    descriptor:  CapabilityDescriptor,
    nativeInput: TNativeInput,
  ): Promise<TNativeOutput>;

  /** Optional: validate request before dispatch. Return error string or null. */
  validateRequest?(req: AgentRequest, descriptor: CapabilityDescriptor): string | null;

  /** Optional: custom error handler. */
  onError?(req: AgentRequest, err: Error): AgentResponse;

  /** Wrap the framework agent and return an IAgent. */
  wrap(agent: TAgent): IAgent;
}

// ── base class (abstract) ─────────────────────────────────────────────────────

export abstract class SentrixPlugin<TAgent, TNativeInput, TNativeOutput>
  implements ISentrixPlugin<TAgent, TNativeInput, TNativeOutput>
{
  constructor(readonly config: PluginConfig) {}

  abstract extractCapabilities(agent: TAgent): CapabilityDescriptor[];
  abstract translateRequest(req: AgentRequest, d: CapabilityDescriptor): TNativeInput;
  abstract translateResponse(result: TNativeOutput, requestId: string): AgentResponse;
  abstract invokeNative(agent: TAgent, d: CapabilityDescriptor, input: TNativeInput): Promise<TNativeOutput>;

  validateRequest(_req: AgentRequest, _d: CapabilityDescriptor): string | null {
    return null;
  }

  onError(req: AgentRequest, err: Error): AgentResponse {
    return {
      requestId:    req.requestId,
      status:       'error',
      errorMessage: `[${err.name}] ${err.message}`,
      timestamp:    Date.now(),
    };
  }

  wrap(agent: TAgent): IAgent {
    const caps = this.extractCapabilities(agent);
    return new WrappedAgent(agent, this, caps, this.config);
  }
}

// ── wrapped agent ─────────────────────────────────────────────────────────────

export class WrappedAgent<TAgent, TNativeInput, TNativeOutput> implements IAgent {
  readonly agentId:     string;
  readonly owner:       string;
  readonly metadataUri: string | undefined;
  readonly metadata:    Record<string, unknown>;

  private readonly capMap: Map<string, CapabilityDescriptor>;

  constructor(
    private readonly agent:   TAgent,
    private readonly plugin:  SentrixPlugin<TAgent, TNativeInput, TNativeOutput>,
    capabilities:             CapabilityDescriptor[],
    config:                   PluginConfig,
  ) {
    this.agentId     = config.agentId;
    this.owner       = config.owner;
    this.metadataUri = config.metadataUri;
    this.metadata    = {
      name:        config.name,
      version:     config.version,
      description: config.description,
      tags:        config.tags,
    };
    this.capMap = new Map(capabilities.map(c => [c.name, c]));
  }

  getCapabilities(): string[] {
    return [...this.capMap.keys()];
  }

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    const descriptor = this.capMap.get(req.capability);
    if (!descriptor) {
      return {
        requestId:    req.requestId,
        status:       'error',
        errorMessage: `Unknown capability: "${req.capability}". Available: ${[...this.capMap.keys()].join(', ')}`,
      };
    }

    const validationError = this.plugin.validateRequest?.(req, descriptor);
    if (validationError) {
      return { requestId: req.requestId, status: 'error', errorMessage: validationError };
    }

    try {
      const nativeInput  = this.plugin.translateRequest(req, descriptor);
      const nativeResult = await this.plugin.invokeNative(this.agent, descriptor, nativeInput);
      return this.plugin.translateResponse(nativeResult, req.requestId);
    } catch (err) {
      return this.plugin.onError!(req, err instanceof Error ? err : new Error(String(err)));
    }
  }

  /**
   * Streaming variant of handleRequest.
   *
   * Default implementation falls back to handleRequest and emits the full
   * result as one StreamChunk followed by a StreamEnd — so every agent
   * supports POST /invoke/stream out of the box.
   *
   * Override in a subclass or framework plugin to yield genuine token chunks.
   */
  async *streamRequest(req: AgentRequest): AsyncIterable<import('../interfaces/IAgentMesh').StreamChunk | import('../interfaces/IAgentMesh').StreamEnd> {
    const resp = await this.handleRequest(req);
    let seq = 0;

    if (resp.status === 'error') {
      yield {
        requestId: req.requestId,
        type:      'end' as const,
        error:     resp.errorMessage,
        sequence:  0,
        timestamp: Date.now(),
      };
      return;
    }

    const content = (resp.result as any)?.content ?? '';
    if (content) {
      yield {
        requestId: req.requestId,
        type:      'chunk' as const,
        delta:     String(content),
        result:    resp.result,
        sequence:  seq++,
        timestamp: Date.now(),
      };
    }

    yield {
      requestId:   req.requestId,
      type:        'end' as const,
      finalResult: resp.result,
      sequence:    seq,
      timestamp:   Date.now(),
    };
  }

  async registerDiscovery(): Promise<void> {
    const { DiscoveryFactory } = await import('../discovery/DiscoveryFactory');
    const registry = DiscoveryFactory.create({
      type: (this.plugin.config.discoveryType as any),
      http: this.plugin.config.discoveryUrl
        ? { baseUrl: this.plugin.config.discoveryUrl, apiKey: this.plugin.config.discoveryKey }
        : undefined,
    });
    const entry: DiscoveryEntry = {
      agentId:      this.agentId,
      name:         this.plugin.config.name,
      owner:        this.owner,
      capabilities: this.getCapabilities(),
      network: {
        protocol: (this.plugin.config.protocol ?? 'http') as any,
        host:     this.plugin.config.host     ?? 'localhost',
        port:     this.plugin.config.port     ?? 6174,
        tls:      this.plugin.config.tls      ?? false,
      },
      health: { status: 'healthy', lastHeartbeat: new Date().toISOString(), uptimeSeconds: 0 },
      registeredAt: new Date().toISOString(),
    };
    await registry.register(entry);
    printStartupBanner(this);
  }

  async unregisterDiscovery(): Promise<void> {
    const { DiscoveryFactory } = await import('../discovery/DiscoveryFactory');
    const registry = DiscoveryFactory.create({ type: (this.plugin.config.discoveryType as any) });
    await registry.unregister(this.agentId);
  }

  // ─── ANR / Identity exposure ───────────────────────────────────────────────

  getAnr(): DiscoveryEntry {
    return {
      agentId:      this.agentId,
      name:         this.plugin.config.name,
      owner:        this.owner ?? 'anonymous',
      capabilities: this.getCapabilities(),
      network: {
        protocol: (this.plugin.config.protocol ?? 'http') as any,
        host:     this.plugin.config.host     ?? 'localhost',
        port:     this.plugin.config.port     ?? 6174,
        tls:      this.plugin.config.tls      ?? false,
      },
      health: { status: 'healthy', lastHeartbeat: new Date().toISOString(), uptimeSeconds: 0 },
      registeredAt: new Date().toISOString(),
      metadataUri:  this.metadataUri,
    };
  }

  async getPeerId(): Promise<string | null> {
    const signingKey = this.plugin.config.signingKey;
    if (!signingKey) return null;
    try {
      const { peerIdFromAnrKey } = await import('../discovery/libp2p/PeerIdFromAnr');
      const raw    = Buffer.from(signingKey.replace(/^0x/, ''), 'hex');
      const peerId = await peerIdFromAnrKey(new Uint8Array(raw));
      return peerId.toString();
    } catch {
      return null;
    }
  }

  /**
   * Start the built-in HTTP server for this agent and block until shutdown.
   *
   * Convenience wrapper around `serve()` from `../server`.  Starts listening,
   * registers with discovery (printing the startup banner), and unregisters
   * cleanly on SIGINT / SIGTERM.
   *
   * @param options.host - Bind address. Default: '0.0.0.0'
   * @param options.port - TCP port. Overridden by SENTRIX_PORT env var. Default: 6174
   *
   * @example
   * ```ts
   * const plugin = new GoogleADKPlugin(config);
   * const agent  = plugin.wrap(myAgent);
   * await agent.serve({ port: 6174 });
   * ```
   */
  async serve(options: { host?: string; port?: number } = {}): Promise<void> {
    const { serve: _serve } = await import('../server');
    await _serve(this, options);
  }
}

// ── startup banner ─────────────────────────────────────────────────────────────

function printStartupBanner(agent: WrappedAgent<unknown, unknown, unknown>): void {
  const cfg  = agent['plugin'].config;
  const caps = agent.getCapabilities();

  const R   = '\x1b[0m';
  const B   = '\x1b[1m';
  const C   = '\x1b[36m';
  const G   = '\x1b[32m';
  const Y   = '\x1b[33m';
  const DIM = '\x1b[2m';
  const line = `${DIM}${'─'.repeat(60)}${R}`;

  const endpoint = `${cfg.tls ? 'https' : (cfg.protocol ?? 'http')}://${cfg.host ?? 'localhost'}:${cfg.port ?? 6174}`;

  const lines: string[] = [
    '',
    line,
    `  ${B}${C}Sentrix Agent Online${R}  ${DIM}v${cfg.version ?? '?'}${R}`,
    line,
    `  ${B}Name       ${R}  ${cfg.name}`,
    `  ${B}Agent ID   ${R}  ${G}${agent.agentId}${R}`,
    ...(cfg.owner && cfg.owner !== 'anonymous' ? [`  ${B}Owner      ${R}  ${cfg.owner}`] : []),
    `  ${B}Endpoint   ${R}  ${endpoint}`,
    `  ${B}Discovery  ${R}  ${cfg.discoveryType ?? 'local'}${cfg.discoveryUrl ? `  →  ${cfg.discoveryUrl}` : ''}`,
    ...(agent.metadataUri ? [`  ${B}Metadata   ${R}  ${agent.metadataUri}`] : []),
    `  ${B}Capabilities${R} (${caps.length})`,
    ...caps.map(cap => `           ${G}•${R} ${cap}`),
    line,
    '',
  ];

  // getPeerId is async — print synchronously without it; peer ID appears after if available
  console.log(lines.join('\n'));

  agent.getPeerId().then(peerId => {
    if (peerId) console.log(`  ${B}${DIM}Peer ID${R}  ${DIM}${peerId}${R}\n`);
  }).catch(() => {/* no key configured */});
}
