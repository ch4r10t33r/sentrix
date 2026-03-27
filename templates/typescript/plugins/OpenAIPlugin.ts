/**
 * OpenAI Agents SDK → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps any OpenAI Agents SDK `Agent` so it appears as a standard Borgkit
 * IAgent on the mesh — discoverable, callable by other agents, and serveable
 * over HTTP via the built-in server.
 *
 * Capability extraction strategy (priority order)
 * ────────────────────────────────────────────────
 *   1. Explicit `capabilityMap` in PluginConfig
 *   2. Agent's `tools` list      — each tool becomes one capability
 *   3. Agent's `handoffs` list   — each sub-agent becomes one capability
 *   4. Single `invoke` fallback  — whole-agent invocation
 *
 * Requirements
 * ────────────
 *   npm install @openai/agents
 *   export OPENAI_API_KEY=sk-...
 *
 * Usage
 * ─────
 * ```ts
 * import { Agent, tool }        from '@openai/agents';
 * import { OpenAIPlugin }       from './plugins/OpenAIPlugin';
 * import { wrapOpenAI }         from './plugins/OpenAIPlugin';
 *
 * const weatherTool = tool({
 *   name:        'get_weather',
 *   description: 'Return the current weather for a city.',
 *   parameters:  z.object({ city: z.string() }),
 *   execute: async ({ city }) => `Sunny, 22°C in ${city}`,
 * });
 *
 * const oaiAgent = new Agent({
 *   name:         'WeatherBot',
 *   instructions: 'Answer weather questions concisely.',
 *   tools:        [weatherTool],
 *   model:        'gpt-4o-mini',
 * });
 *
 * // Verbose:
 * const plugin = new OpenAIPlugin({ agentId: 'borgkit://agent/weather', name: 'WeatherBot', ... });
 * const agent  = plugin.wrap(oaiAgent);
 *
 * // One-liner:
 * const agent = wrapOpenAI(oaiAgent, { agentId: 'borgkit://agent/weather', name: 'WeatherBot', ... });
 *
 * await agent.serve({ port: 8082 });
 * ```
 */

import { AgentRequest }  from '../interfaces/IAgentRequest';
import { AgentResponse } from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor, WrappedAgent } from './IPlugin';

// ── extended config ────────────────────────────────────────────────────────────

export interface OpenAIPluginConfig extends PluginConfig {
  /** Expose each tool as a separate Borgkit capability (default: true). */
  exposeToolsAsCapabilities?:    boolean;
  /** Expose handoff targets as capabilities (default: true). */
  exposeHandoffsAsCapabilities?: boolean;
  /** Max agentic turns per request (default: 10). */
  maxTurns?:                     number;
  /** Optional model override — useful for cheaper test deployments. */
  modelOverride?:                string;
}

// ── native I/O ─────────────────────────────────────────────────────────────────

interface OpenAINativeInput {
  message:          string;
  targetAgentName?: string;
}

// ── plugin ─────────────────────────────────────────────────────────────────────

const HANDOFF_PREFIX = '__handoff__:';

export class OpenAIPlugin extends BorgkitPlugin<unknown, OpenAINativeInput, unknown> {
  private readonly oaiConfig: Required<
    Pick<OpenAIPluginConfig,
      'exposeToolsAsCapabilities' |
      'exposeHandoffsAsCapabilities' |
      'maxTurns'
    >
  > & OpenAIPluginConfig;

  constructor(config: OpenAIPluginConfig) {
    super(config);
    this.oaiConfig = {
      exposeToolsAsCapabilities:    true,
      exposeHandoffsAsCapabilities: true,
      maxTurns:                     10,
      ...config,
    } as typeof this.oaiConfig;
  }

  // ── capability extraction ───────────────────────────────────────────────────

  extractCapabilities(agent: unknown): CapabilityDescriptor[] {
    const caps: CapabilityDescriptor[] = [];
    const a = agent as Record<string, unknown>;

    // 1. Explicit capabilityMap
    if (this.config.capabilityMap && Object.keys(this.config.capabilityMap).length > 0) {
      return Object.entries(this.config.capabilityMap).map(([borgkitName, nativeName]) => ({
        name:        borgkitName,
        description: `Mapped capability → ${nativeName}`,
        nativeName,
      }));
    }

    // 2. Tools
    if (this.oaiConfig.exposeToolsAsCapabilities) {
      const tools = getTools(a);
      for (const t of tools) {
        caps.push(toolToDescriptor(t));
      }
    }

    // 3. Handoffs
    if (this.oaiConfig.exposeHandoffsAsCapabilities) {
      for (const handoff of getHandoffs(a)) {
        const target = unwrapHandoff(handoff);
        if (!target) continue;
        const name = sanitize((target as Record<string, unknown>)['name'] as string ?? 'handoff');
        const instructions = (target as Record<string, unknown>)['instructions'] as string ?? '';
        caps.push({
          name,
          description: instructions.slice(0, 120) || `Handoff to sub-agent '${name}'`,
          nativeName:  `${HANDOFF_PREFIX}${name}`,
          tags:        ['handoff', 'sub-agent'],
        });
      }
    }

    // 4. Fallback: single invoke capability
    if (caps.length === 0) {
      const instructions = (a['instructions'] as string) ?? '';
      caps.push({
        name:        'invoke',
        description: instructions.slice(0, 120) || `Invoke the ${this.config.name} agent`,
        nativeName:  '__agent__',
        tags:        this.config.tags ?? [],
      });
    }

    return caps;
  }

  // ── request translation ─────────────────────────────────────────────────────

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): OpenAINativeInput {
    const native = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;
    const payload = req.payload as Record<string, unknown>;

    // Whole-agent / generic
    if (native === '__agent__') {
      const message = String(
        payload['message'] ?? payload['input'] ?? payload['query'] ?? JSON.stringify(payload)
      );
      return { message };
    }

    // Handoff target
    if (native.startsWith(HANDOFF_PREFIX)) {
      const targetName = native.slice(HANDOFF_PREFIX.length);
      const message = String(
        payload['message'] ?? payload['input'] ?? JSON.stringify(payload)
      );
      return { message, targetAgentName: targetName };
    }

    // Tool-specific: craft a prompt
    const argsStr = JSON.stringify(payload, null, 2);
    return { message: `Use the \`${native}\` function with these arguments:\n${argsStr}` };
  }

  // ── response translation ────────────────────────────────────────────────────

  translateResponse(nativeResult: unknown, requestId: string): AgentResponse {
    try {
      const content = extractOutput(nativeResult);
      return {
        requestId,
        status:    'success',
        result:    { content, raw: safeSerialize(nativeResult) },
        timestamp: Date.now(),
      };
    } catch (err) {
      return {
        requestId,
        status:       'error',
        errorMessage: `Response translation failed: ${err}`,
      };
    }
  }

  // ── native invocation ───────────────────────────────────────────────────────

  async invokeNative(
    agent:        unknown,
    _descriptor:  CapabilityDescriptor,
    nativeInput:  OpenAINativeInput,
  ): Promise<unknown> {
    let { run } = await import('@openai/agents').catch(() => {
      throw new Error(
        'openai-agents is not installed.\n' +
        'Install: npm install @openai/agents\n' +
        'Also set: OPENAI_API_KEY=sk-...'
      );
    });

    let runAgent = agent;

    // Resolve handoff target
    if (nativeInput.targetAgentName) {
      const a = agent as Record<string, unknown>;
      const handoffs = getHandoffs(a);
      let found = false;
      for (const h of handoffs) {
        const target = unwrapHandoff(h);
        if (target && sanitize((target as Record<string, unknown>)['name'] as string ?? '') === nativeInput.targetAgentName) {
          runAgent = target;
          found = true;
          break;
        }
      }
      if (!found) {
        const available = handoffs
          .map(h => unwrapHandoff(h))
          .filter(Boolean)
          .map(t => sanitize((t as Record<string, unknown>)['name'] as string ?? ''));
        throw new Error(
          `Handoff target '${nativeInput.targetAgentName}' not found. ` +
          `Available: [${available.join(', ')}]`
        );
      }
    }

    // Apply model override if configured
    if (this.oaiConfig.modelOverride) {
      try {
        const agentObj = runAgent as Record<string, unknown>;
        if (typeof agentObj['clone'] === 'function') {
          runAgent = (agentObj['clone'] as Function)({ model: this.oaiConfig.modelOverride });
        }
      } catch { /* clone not supported in this SDK version */ }
    }

    const result = await run(runAgent as never, nativeInput.message, {
      maxTurns: this.oaiConfig.maxTurns,
    });

    return result;
  }
}

// ── helpers ────────────────────────────────────────────────────────────────────

function getTools(agent: Record<string, unknown>): unknown[] {
  return Array.isArray(agent['tools']) ? agent['tools'] : [];
}

function getHandoffs(agent: Record<string, unknown>): unknown[] {
  return Array.isArray(agent['handoffs']) ? agent['handoffs'] : [];
}

function unwrapHandoff(handoff: unknown): unknown | null {
  if (!handoff) return null;
  const h = handoff as Record<string, unknown>;
  // Direct Agent
  if (h['name'] && h['tools'] !== undefined) return handoff;
  // Handoff wrapper object
  if (h['agent']) return h['agent'];
  // Callable
  if (typeof handoff === 'function') {
    try { return (handoff as Function)(); } catch { return null; }
  }
  return null;
}

function toolToDescriptor(tool: unknown): CapabilityDescriptor {
  const t = tool as Record<string, unknown>;
  const name = sanitize((t['name'] as string) ?? String(tool));
  const desc = (t['description'] as string) ?? '';

  // @openai/agents FunctionTool stores schema in .parameters (zod schema → JSON schema)
  let inputSchema: Record<string, unknown> | undefined;
  for (const attr of ['parameters', 'params_json_schema', 'schema', 'inputSchema']) {
    const s = t[attr];
    if (s && typeof s === 'object' && !Array.isArray(s)) {
      inputSchema = s as Record<string, unknown>;
      break;
    }
  }

  return { name, description: desc, nativeName: (t['name'] as string) ?? name, inputSchema };
}

function extractOutput(result: unknown): string {
  if (result == null) return '';
  const r = result as Record<string, unknown>;
  for (const attr of ['finalOutput', 'final_output', 'output', 'response', 'content', 'text']) {
    if (r[attr] != null) return String(r[attr]);
  }
  return String(result);
}

function sanitize(name: string): string {
  return (name ?? '').replace(/\s+/g, '_').replace(/-/g, '_').toLowerCase();
}

function safeSerialize(obj: unknown): unknown {
  try { return JSON.parse(JSON.stringify(obj)); } catch { return String(obj); }
}

// ── one-liner convenience wrapper ─────────────────────────────────────────────

/**
 * Wrap an OpenAI Agents SDK Agent for the Borgkit mesh in one line.
 *
 * @example
 * ```ts
 * import { Agent }    from '@openai/agents';
 * import { wrapOpenAI } from './plugins/OpenAIPlugin';
 *
 * const agent = wrapOpenAI(
 *   new Agent({ name: 'WeatherBot', tools: [weatherTool] }),
 *   { agentId: 'borgkit://agent/weather', name: 'WeatherBot', owner: '0x...' },
 * );
 * await agent.serve({ port: 8082 });
 * ```
 */
export function wrapOpenAI(
  agent:  unknown,
  config: OpenAIPluginConfig,
): WrappedAgent<unknown, OpenAINativeInput, unknown> {
  return new OpenAIPlugin(config).wrap(agent);
}
