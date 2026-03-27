/**
 * Google ADK → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps a Google ADK `BaseAgent` or `LlmAgent` so it is fully discoverable
 * and callable on the Borgkit mesh.
 *
 * Capability extraction (priority order):
 *   1. Explicit capabilityMap in config
 *   2. agent.tools list (FunctionTool / BaseTool instances)
 *   3. agent.subAgents (multi-agent setup)
 *   4. Single 'invoke' fallback (whole-agent invocation)
 *
 * Usage:
 *   import { wrapGoogleADK } from './plugins/GoogleADKPlugin';
 *
 *   const agent = new LlmAgent({ name: 'Support', model: 'gemini-2.0-flash', tools: [...] });
 *   const borgkitAgent = wrapGoogleADK(agent, {
 *     agentId: 'borgkit://agent/support',
 *     name:    'SupportAgent',
 *     owner:   '0xYourWallet',
 *   });
 *   await borgkitAgent.registerDiscovery();
 *
 * Install: npm install @google/adk
 */

import { AgentRequest }        from '../interfaces/IAgentRequest';
import { AgentResponse }       from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

// ── extended config ───────────────────────────────────────────────────────────

export interface GoogleADKPluginConfig extends PluginConfig {
  /** ADK app name used when creating a Runner session (default: 'borgkit') */
  appName?:  string;
  /** ADK user ID for session creation (default: 'borgkit-user') */
  userId?:   string;
  /** Expose each tool as a separate Borgkit capability (default: true) */
  exposeToolsAsCapabilities?: boolean;
  /** Expose sub-agents as capabilities (default: false) */
  exposeSubAgents?: boolean;
}

// ── native I/O types ──────────────────────────────────────────────────────────

interface ADKNativeInput {
  message:  string;
  toolName: string;
  payload:  Record<string, unknown>;
  fromId:   string;
}

// ── plugin ────────────────────────────────────────────────────────────────────

export class GoogleADKPlugin extends BorgkitPlugin<unknown, ADKNativeInput, unknown[]> {
  private readonly adkConfig: Required<Pick<GoogleADKPluginConfig,
    'appName' | 'userId' | 'exposeToolsAsCapabilities' | 'exposeSubAgents'>>;

  constructor(config: GoogleADKPluginConfig) {
    super(config);
    this.adkConfig = {
      appName:                  config.appName  ?? 'borgkit',
      userId:                   config.userId   ?? 'borgkit-user',
      exposeToolsAsCapabilities: config.exposeToolsAsCapabilities !== false,
      exposeSubAgents:          config.exposeSubAgents ?? false,
    };
  }

  // ── capability extraction ──────────────────────────────────────────────────

  extractCapabilities(agent: unknown): CapabilityDescriptor[] {
    const caps: CapabilityDescriptor[] = [];
    const a = agent as Record<string, unknown>;

    if (this.adkConfig.exposeToolsAsCapabilities) {
      const tools = this._getTools(a);
      caps.push(...tools.map(t => this._toolToDescriptor(t)));
    }

    if (this.adkConfig.exposeSubAgents) {
      const subs = (a['subAgents'] as unknown[] | undefined) ?? [];
      for (const sub of subs) {
        const s = sub as Record<string, unknown>;
        caps.push({
          name:        this._sanitize(String(s['name'] ?? 'subagent')),
          description: String(s['description'] ?? ''),
          nativeName:  String(s['name'] ?? ''),
          tags:        ['sub-agent'],
        });
      }
    }

    if (caps.length === 0) {
      caps.push({
        name:        'invoke',
        description: String(a['description'] ?? this.config.description ?? ''),
        nativeName:  '__agent__',
        tags:        this.config.tags ?? [],
      });
    }

    return caps;
  }

  private _getTools(agent: Record<string, unknown>): unknown[] {
    for (const attr of ['tools', '_tools', 'canonicalTools']) {
      const t = agent[attr];
      if (Array.isArray(t) && t.length > 0) return t;
    }
    return [];
  }

  private _toolToDescriptor(tool: unknown): CapabilityDescriptor {
    const t    = tool as Record<string, unknown>;
    const name = String(t['name'] ?? t['__name__'] ?? 'unknown');
    const desc = String(t['description'] ?? '');

    // Try to extract JSON schema from FunctionTool declaration
    let inputSchema: Record<string, unknown> | undefined;
    try {
      const decl = (t['getDeclaration'] as (() => unknown) | undefined)?.();
      const params = (decl as Record<string, unknown> | undefined)?.['parameters'];
      if (params && typeof params === 'object') {
        inputSchema = { type: 'object', properties: params as Record<string, unknown> };
      }
    } catch { /* non-fatal */ }

    return {
      name:        this._sanitize(name),
      description: desc,
      nativeName:  name,
      inputSchema,
      tags:        [],
    };
  }

  private _sanitize(name: string): string {
    return name.replace(/\s+/g, '_').replace(/-/g, '_').toLowerCase();
  }

  // ── request translation ────────────────────────────────────────────────────

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): ADKNativeInput {
    const native  = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;
    const payload = req.payload ?? {};

    const message = native === '__agent__'
      ? (String(payload['message'] ?? payload['input'] ?? payload['query'] ?? JSON.stringify(payload)))
      : `Call the tool \`${native}\` with these arguments:\n${JSON.stringify(payload, null, 2)}`;

    return { message, toolName: native, payload, fromId: req.from };
  }

  // ── response translation ───────────────────────────────────────────────────

  translateResponse(events: unknown[], requestId: string): AgentResponse {
    try {
      const parts: string[] = [];
      for (const evt of events) {
        const text = this._extractEventText(evt);
        if (text) parts.push(text);
      }
      return {
        requestId,
        status:    'success',
        result:    { content: parts.join('\n'), events: events.length },
        timestamp: Date.now(),
      };
    } catch (e) {
      return {
        requestId,
        status:       'error',
        errorMessage: String(e),
        timestamp:    Date.now(),
      };
    }
  }

  private _extractEventText(event: unknown): string | null {
    const e = event as Record<string, unknown>;
    const content = e['content'] as Record<string, unknown> | undefined;
    if (!content) return null;
    const parts = content['parts'] as Array<Record<string, unknown>> | undefined;
    if (!parts) return null;
    return parts.map(p => String(p['text'] ?? '')).filter(Boolean).join(' ') || null;
  }

  // ── native invocation ──────────────────────────────────────────────────────

  async invokeNative(
    agent:      unknown,
    _descriptor: CapabilityDescriptor,
    input:      ADKNativeInput,
  ): Promise<unknown[]> {
    // Dynamic import — soft dependency
    let Runner: any, InMemorySessionService: any, types: any;
    try {
      ({ Runner } = await import('@google/adk/runners'));
      ({ InMemorySessionService } = await import('@google/adk/sessions'));
      types = await import('@google/adk/types');
    } catch (e) {
      throw new Error(
        `@google/adk not installed or import failed: ${e}\n` +
        'Install with: npm install @google/adk'
      );
    }

    const sessionSvc = new InMemorySessionService();
    const runner     = new Runner({
      agent,
      appName:        this.adkConfig.appName,
      sessionService: sessionSvc,
    });

    const session = await sessionSvc.createSession({
      appName: this.adkConfig.appName,
      userId:  this.adkConfig.userId,
    });

    const userMsg = new types.Content({
      role:  'user',
      parts: [new types.Part({ text: input.message })],
    });

    const events: unknown[] = [];
    for await (const event of runner.runAsync({
      userId:     this.adkConfig.userId,
      sessionId:  session.id,
      newMessage: userMsg,
    })) {
      events.push(event);
    }
    return events;
  }
}

// ── convenience helper ────────────────────────────────────────────────────────

export function wrapGoogleADK(
  agent:   unknown,
  config:  GoogleADKPluginConfig,
): ReturnType<GoogleADKPlugin['wrap']> {
  return new GoogleADKPlugin(config).wrap(agent);
}
