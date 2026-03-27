/**
 * smolagents → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps a smolagents agent (CodeAgent or ToolCallingAgent) so its
 * tools become Borgkit capabilities.
 *
 * Usage:
 *   import { wrapSmolagents } from './plugins/SmolagentsPlugin';
 *
 *   // smolagents JS (or Python via a bridge)
 *   const agent = new ToolCallingAgent({ tools: [webSearch], model });
 *   const borgkit = wrapSmolagents(agent, {
 *     agentId: 'borgkit://agent/researcher',
 *     name:    'ResearchAgent',
 *     owner:   '0xYourWallet',
 *   });
 *   await borgkit.registerDiscovery();
 *
 * Install: npm install smolagents  (or the JS equivalent)
 */

import { AgentRequest }        from '../interfaces/IAgentRequest';
import { AgentResponse }       from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

export interface SmolagentsPluginConfig extends PluginConfig {
  /** Additional kwargs forwarded to agent.run() */
  runKwargs?: Record<string, unknown>;
  /** Maximum number of agent steps (default: 10) */
  maxSteps?: number;
}

interface SmolagentsNativeInput {
  task:    string;
  payload: Record<string, unknown>;
}

export class SmolagentsPlugin extends BorgkitPlugin<unknown, SmolagentsNativeInput, string> {
  private readonly smaConfig: Required<Pick<SmolagentsPluginConfig, 'runKwargs' | 'maxSteps'>>;

  constructor(config: SmolagentsPluginConfig) {
    super(config);
    this.smaConfig = {
      runKwargs: config.runKwargs ?? {},
      maxSteps:  config.maxSteps  ?? 10,
    };
  }

  extractCapabilities(agent: unknown): CapabilityDescriptor[] {
    const a = agent as Record<string, unknown>;

    // smolagents stores tools in .toolbox or .tools
    const toolbox = (a['toolbox'] as Record<string, unknown> | undefined);
    const toolsMap = toolbox?.['tools'] as Record<string, unknown> | undefined;
    const toolsArr = Array.isArray(a['tools']) ? a['tools'] as unknown[] : null;

    const tools: Array<[string, unknown]> = toolsMap
      ? Object.entries(toolsMap)
      : (toolsArr?.map((t, i) => [String(i), t]) ?? []);

    if (tools.length > 0) {
      return tools.map(([, t]) => {
        const tool = t as Record<string, unknown>;
        const name = String(tool['name'] ?? 'tool');
        const desc = String(tool['description'] ?? '');

        // smolagents tools have .inputs (dict of param → {type, description})
        let inputSchema: Record<string, unknown> | undefined;
        const inputs = tool['inputs'] as Record<string, { type: string; description?: string }> | undefined;
        if (inputs) {
          inputSchema = {
            type: 'object',
            properties: Object.fromEntries(
              Object.entries(inputs).map(([k, v]) => [k, { type: v.type, description: v.description }])
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

    // Fallback: whole-agent invocation
    return [{
      name:        'run',
      description: String(a['description'] ?? this.config.description ?? ''),
      nativeName:  '__run__',
      tags:        this.config.tags ?? [],
    }];
  }

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): SmolagentsNativeInput {
    const payload = req.payload ?? {};
    const native  = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;

    const task = native === '__run__'
      ? String(payload['task'] ?? payload['message'] ?? payload['input'] ?? payload['query'] ?? JSON.stringify(payload))
      : `Use the ${native} tool. Args: ${JSON.stringify(payload)}`;

    return { task, payload };
  }

  translateResponse(result: string, requestId: string): AgentResponse {
    return {
      requestId,
      status:    'success',
      result:    { output: result },
      timestamp: Date.now(),
    };
  }

  async invokeNative(
    agent:      unknown,
    _descriptor: CapabilityDescriptor,
    input:      SmolagentsNativeInput,
  ): Promise<string> {
    const a = agent as { run: (task: string, opts?: Record<string, unknown>) => unknown };
    if (typeof a.run !== 'function') {
      throw new Error('smolagents agent must have a .run(task) method');
    }

    const result = await Promise.resolve(
      a.run(input.task, { maxSteps: this.smaConfig.maxSteps, ...this.smaConfig.runKwargs })
    );

    return String(result ?? '');
  }
}

export function wrapSmolagents(
  agent:  unknown,
  config: SmolagentsPluginConfig,
): ReturnType<SmolagentsPlugin['wrap']> {
  return new SmolagentsPlugin(config).wrap(agent);
}
