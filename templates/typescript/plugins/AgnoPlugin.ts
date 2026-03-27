/**
 * Agno → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps an Agno `Agent` so its tools become Borgkit capabilities.
 *
 * Usage:
 *   import { wrapAgno } from './plugins/AgnoPlugin';
 *   import { Agent }    from 'agno';
 *
 *   const agent = new Agent({ model: ..., tools: [webSearch], description: '...' });
 *   const borgkit = wrapAgno(agent, {
 *     agentId: 'borgkit://agent/researcher',
 *     name:    'ResearchAgent',
 *     owner:   '0xYourWallet',
 *   });
 *   await borgkit.registerDiscovery();
 *
 * Install: npm install agno
 */

import { AgentRequest }        from '../interfaces/IAgentRequest';
import { AgentResponse }       from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

export interface AgnoPluginConfig extends PluginConfig {
  /** Stream Agno response (default: false — collect full output) */
  stream?: boolean;
  /** Pass markdown=true to agent.run() (default: false) */
  markdown?: boolean;
}

interface AgnoNativeInput {
  message:  string;
  payload:  Record<string, unknown>;
}

export class AgnoPlugin extends BorgkitPlugin<unknown, AgnoNativeInput, string> {
  private readonly agnoConfig: Required<Pick<AgnoPluginConfig, 'stream' | 'markdown'>>;

  constructor(config: AgnoPluginConfig) {
    super(config);
    this.agnoConfig = {
      stream:   config.stream   ?? false,
      markdown: config.markdown ?? false,
    };
  }

  extractCapabilities(agent: unknown): CapabilityDescriptor[] {
    const a = agent as Record<string, unknown>;

    // Agno Agent exposes tools via .tools or ._tools
    const tools: unknown[] = [];
    for (const attr of ['tools', '_tools']) {
      const t = a[attr];
      if (Array.isArray(t) && t.length > 0) { tools.push(...t); break; }
    }

    if (tools.length > 0) {
      return tools.map(t => {
        const tool = t as Record<string, unknown>;
        const fn   = (tool['entrypoint'] ?? tool['func'] ?? tool) as Record<string, unknown>;
        const name = String(fn['name'] ?? tool['name'] ?? 'tool');
        const desc = String(fn['description'] ?? tool['description'] ?? '');

        // Extract JSON schema from Python-style __annotations__ or schema property
        let inputSchema: Record<string, unknown> | undefined;
        const annotations = fn['__annotations__'] as Record<string, string> | undefined;
        if (annotations) {
          inputSchema = {
            type: 'object',
            properties: Object.fromEntries(
              Object.keys(annotations)
                .filter(k => k !== 'return')
                .map(k => [k, { type: 'string' }])
            ),
          };
        }

        return {
          name:        name.replace(/\s+/g, '_').toLowerCase(),
          description: desc,
          nativeName:  name,
          inputSchema,
          tags:        [],
        };
      });
    }

    // Fallback: single invoke capability
    return [{
      name:        'invoke',
      description: String(a['description'] ?? this.config.description ?? ''),
      nativeName:  '__agent__',
      tags:        this.config.tags ?? [],
    }];
  }

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): AgnoNativeInput {
    const payload = req.payload ?? {};
    const native  = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;

    const message = native === '__agent__'
      ? String(payload['message'] ?? payload['input'] ?? payload['query'] ?? JSON.stringify(payload))
      : `Call ${native} with: ${JSON.stringify(payload)}`;

    return { message, payload };
  }

  translateResponse(result: string, requestId: string): AgentResponse {
    return {
      requestId,
      status:    'success',
      result:    { content: result },
      timestamp: Date.now(),
    };
  }

  async invokeNative(
    agent:      unknown,
    _descriptor: CapabilityDescriptor,
    input:      AgnoNativeInput,
  ): Promise<string> {
    let AgnoAgent: any;
    try {
      ({ Agent: AgnoAgent } = await import('agno'));
    } catch (e) {
      throw new Error(`agno not installed: ${e}\nInstall: npm install agno`);
    }

    const a = agent as { run: (msg: string, opts?: Record<string, unknown>) => unknown };
    const result = await Promise.resolve(
      a.run(input.message, { stream: this.agnoConfig.stream, markdown: this.agnoConfig.markdown })
    );

    // Agno RunResponse has .content
    if (result && typeof result === 'object') {
      const r = result as Record<string, unknown>;
      return String(r['content'] ?? r['text'] ?? r['output'] ?? JSON.stringify(result));
    }
    return String(result ?? '');
  }
}

export function wrapAgno(
  agent:  unknown,
  config: AgnoPluginConfig,
): ReturnType<AgnoPlugin['wrap']> {
  return new AgnoPlugin(config).wrap(agent);
}
