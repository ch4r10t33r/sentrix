/**
 * LlamaIndex → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps a LlamaIndex agent (OpenAIAgent, ReActAgent, FunctionCallingAgent)
 * so its tools become Borgkit capabilities.
 *
 * Usage:
 *   import { wrapLlamaIndex }           from './plugins/LlamaIndexPlugin';
 *   import { OpenAIAgent, FunctionTool } from 'llamaindex';
 *
 *   const tool  = FunctionTool.from(webSearch, { name: 'web_search', description: '...' });
 *   const agent = new OpenAIAgent({ tools: [tool] });
 *
 *   const borgkit = wrapLlamaIndex(agent, {
 *     agentId: 'borgkit://agent/researcher',
 *     name:    'ResearchAgent',
 *     owner:   '0xYourWallet',
 *     tools:   [tool],
 *   });
 *   await borgkit.registerDiscovery();
 *
 * Install: npm install llamaindex
 */

import { AgentRequest }        from '../interfaces/IAgentRequest';
import { AgentResponse }       from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

export interface LlamaIndexPluginConfig extends PluginConfig {
  /** Explicit tool list for capability extraction (preferred over introspection) */
  tools?: unknown[];
  /** Use agent.chat() instead of agent.query() (default: false) */
  useChat?: boolean;
}

interface LlamaIndexNativeInput {
  message: string;
  payload: Record<string, unknown>;
}

export class LlamaIndexPlugin extends BorgkitPlugin<unknown, LlamaIndexNativeInput, string> {
  private readonly liConfig: Required<Pick<LlamaIndexPluginConfig, 'tools' | 'useChat'>>;

  constructor(config: LlamaIndexPluginConfig) {
    super(config);
    this.liConfig = {
      tools:   config.tools   ?? [],
      useChat: config.useChat ?? false,
    };
  }

  extractCapabilities(agent: unknown): CapabilityDescriptor[] {
    // Use explicit tools if provided
    let tools = this.liConfig.tools;

    // Otherwise try to introspect the agent
    if (tools.length === 0) {
      const a = agent as Record<string, unknown>;
      for (const attr of ['tools', '_tools', 'toolRetriever']) {
        const t = a[attr];
        if (Array.isArray(t) && t.length > 0) { tools = t; break; }
      }
    }

    if (tools.length > 0) {
      return tools.map(t => {
        const tool     = t as Record<string, unknown>;
        const metadata = tool['metadata'] as Record<string, unknown> | undefined;
        const name     = String(metadata?.['name'] ?? tool['name'] ?? 'tool');
        const desc     = String(metadata?.['description'] ?? tool['description'] ?? '');
        const schema   = metadata?.['fnSchema'] ?? metadata?.['parameters'];

        return {
          name:        name.replace(/\s+/g, '_').toLowerCase(),
          description: desc,
          nativeName:  name,
          inputSchema: schema ? (schema as Record<string, unknown>) : undefined,
          tags:        [],
        };
      });
    }

    // Fallback
    return [{
      name:        'query',
      description: String((agent as Record<string, unknown>)['description'] ?? this.config.description ?? ''),
      nativeName:  '__query__',
      tags:        this.config.tags ?? [],
    }];
  }

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): LlamaIndexNativeInput {
    const payload = req.payload ?? {};
    const native  = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;

    const message = native === '__query__'
      ? String(payload['query'] ?? payload['message'] ?? payload['input'] ?? JSON.stringify(payload))
      : `Use the ${native} tool with: ${JSON.stringify(payload)}`;

    return { message, payload };
  }

  translateResponse(result: string, requestId: string): AgentResponse {
    return {
      requestId,
      status:    'success',
      result:    { response: result },
      timestamp: Date.now(),
    };
  }

  async invokeNative(
    agent:      unknown,
    _descriptor: CapabilityDescriptor,
    input:      LlamaIndexNativeInput,
  ): Promise<string> {
    const a = agent as Record<string, (...args: unknown[]) => unknown>;

    let result: unknown;
    if (this.liConfig.useChat && typeof a['chat'] === 'function') {
      result = await a['chat'](input.message);
    } else if (typeof a['query'] === 'function') {
      result = await a['query'](input.message);
    } else if (typeof a['chat'] === 'function') {
      result = await a['chat'](input.message);
    } else {
      throw new Error('LlamaIndex agent has no .query() or .chat() method');
    }

    if (result && typeof result === 'object') {
      const r = result as Record<string, unknown>;
      return String(r['response'] ?? r['message'] ?? r['content'] ?? r['text'] ?? JSON.stringify(result));
    }
    return String(result ?? '');
  }
}

export function wrapLlamaIndex(
  agent:  unknown,
  config: LlamaIndexPluginConfig,
): ReturnType<LlamaIndexPlugin['wrap']> {
  return new LlamaIndexPlugin(config).wrap(agent);
}
