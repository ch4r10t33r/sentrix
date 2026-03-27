/**
 * LangGraph.js → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps a compiled LangGraph `CompiledGraph` (or any object with `.invoke()`)
 * so it appears as a standard Borgkit IAgent on the mesh.
 *
 * Capability extraction strategy (priority order):
 *   1. Explicit `capabilityMap` in config
 *   2. Tools bound on the graph's agent node  (.nodes["agent"].bound.tools)
 *   3. `tools` passed directly to LangGraphPlugin constructor
 *   4. Single `invoke` capability (whole-graph fallback)
 *
 * Usage:
 *   import { LangGraphPlugin } from './plugins/LangGraphPlugin';
 *
 *   const plugin = new LangGraphPlugin({
 *     agentId: 'borgkit://agent/researcher',
 *     name:    'ResearchAgent',
 *     version: '1.0.0',
 *     tags:    ['research', 'web'],
 *     exposeToolsAsCapabilities: true,
 *   });
 *
 *   const agent = plugin.wrap(compiledGraph);
 *   await agent.registerDiscovery();
 *
 * Install: npm install @langchain/langgraph @langchain/core
 */

import { AgentRequest }         from '../interfaces/IAgentRequest';
import { AgentResponse }        from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

// ── extended config ───────────────────────────────────────────────────────────

export interface LangGraphPluginConfig extends PluginConfig {
  /** Expose each tool as a separate capability (default: true) */
  exposeToolsAsCapabilities?: boolean;
  /** LangGraph state key for user input messages (default: 'messages') */
  inputKey?: string;
  /** LangGraph state key for output messages (default: 'messages') */
  outputKey?: string;
  /** Graph node name to inspect for bound tools (default: 'agent') */
  agentNodeName?: string;
  /** LangGraph recursion limit (default: 25) */
  recursionLimit?: number;
  /** Stream output instead of single invoke (default: false) */
  stream?: boolean;
}

// ── native input / output types ───────────────────────────────────────────────

interface LangGraphInput {
  [key: string]: unknown;
  __borgkitRequestId__: string;
  __borgkitCapability__: string;
}

interface LangGraphOutput {
  [key: string]: unknown;
}

// ── plugin ────────────────────────────────────────────────────────────────────

export class LangGraphPlugin extends BorgkitPlugin<unknown, LangGraphInput, LangGraphOutput> {
  private readonly lgConfig: Required<LangGraphPluginConfig>;
  private readonly explicitTools: unknown[];

  constructor(config: LangGraphPluginConfig, tools: unknown[] = []) {
    super(config);
    this.lgConfig = {
      exposeToolsAsCapabilities: true,
      inputKey:       'messages',
      outputKey:      'messages',
      agentNodeName:  'agent',
      recursionLimit: 25,
      stream:         false,
      host:           'localhost',
      port:           6174,
      protocol:       'http',
      tls:            false,
      discoveryType:  'local',
      tags:           [],
      capabilityMap:  {},
      ...config,
    } as Required<LangGraphPluginConfig>;
    this.explicitTools = tools;
  }

  // ── capability extraction ──────────────────────────────────────────────────

  extractCapabilities(graph: unknown): CapabilityDescriptor[] {
    if (this.lgConfig.exposeToolsAsCapabilities) {
      const tools = this.discoverTools(graph);
      if (tools.length > 0) {
        return tools.map(t => this.toolToDescriptor(t));
      }
    }

    // Single graph invocation fallback
    return [{
      name:        'invoke',
      description: this.config.description ?? 'Invoke the LangGraph agent',
      nativeName:  '__graph__',
      tags:        this.config.tags ?? [],
    }];
  }

  private discoverTools(graph: unknown): unknown[] {
    if (this.explicitTools.length > 0) return this.explicitTools;

    const g = graph as Record<string, unknown>;

    // Check graph.nodes["agent"].bound.tools or .tools
    try {
      const nodes = g['nodes'] as Record<string, unknown> | undefined;
      if (nodes) {
        const node = nodes[this.lgConfig.agentNodeName] as Record<string, unknown> | undefined;
        if (node) {
          for (const attr of ['bound', 'runnable']) {
            const inner = node[attr] as Record<string, unknown> | undefined;
            if (inner?.['tools']) return inner['tools'] as unknown[];
          }
          if (node['tools']) return node['tools'] as unknown[];
        }
      }
    } catch { /* ignore */ }

    // Direct graph.tools
    if (g['tools']) return g['tools'] as unknown[];

    return [];
  }

  private toolToDescriptor(tool: unknown): CapabilityDescriptor {
    const t = tool as Record<string, unknown>;
    const name = (t['name'] as string) ?? String(tool);
    const desc = (t['description'] as string) ?? '';

    let inputSchema: Record<string, unknown> | undefined;
    try {
      const schemaModel = t['schema'] as Record<string, unknown> | undefined;
      if (schemaModel) inputSchema = schemaModel as Record<string, unknown>;
    } catch { /* ignore */ }

    return { name, description: desc, nativeName: name, inputSchema };
  }

  // ── request translation ────────────────────────────────────────────────────

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): LangGraphInput {
    const native = this.config.capabilityMap?.[req.capability] ?? descriptor.nativeName;

    if (native === '__graph__') {
      const content = (req.payload['message'] ?? req.payload['input'] ?? JSON.stringify(req.payload)) as string;
      return {
        [this.lgConfig.inputKey]: [{ role: 'human', content }],
        __borgkitRequestId__:    req.requestId,
        __borgkitCapability__:   req.capability,
      };
    }

    // Tool-specific: craft a prompt containing the tool call args
    const argsStr = JSON.stringify(req.payload, null, 2);
    return {
      [this.lgConfig.inputKey]: [{
        role:    'human',
        content: `Call tool \`${native}\` with:\n${argsStr}`,
      }],
      __borgkitRequestId__:  req.requestId,
      __borgkitCapability__: req.capability,
      __borgkitTool__:       native,
    };
  }

  // ── response translation ───────────────────────────────────────────────────

  translateResponse(result: LangGraphOutput, requestId: string): AgentResponse {
    try {
      const messages = result[this.lgConfig.outputKey] as unknown[];
      const content  = messages ? this.extractContent(messages) : String(result);
      return { requestId, status: 'success', result: { content, raw: result }, timestamp: Date.now() };
    } catch (e) {
      return { requestId, status: 'error', errorMessage: `Response translation failed: ${e}` };
    }
  }

  private extractContent(messages: unknown[]): string {
    // Walk backwards to find the last AI message
    for (let i = messages.length - 1; i >= 0; i--) {
      const msg = messages[i] as Record<string, unknown>;
      const type = msg['type'] as string | undefined;
      if (type === 'ai' || type === 'AIMessage' || msg['role'] === 'assistant') {
        const content = msg['content'];
        if (typeof content === 'string') return content;
        if (Array.isArray(content)) {
          return content.map(c => (c as Record<string, unknown>)['text'] ?? String(c)).join(' ');
        }
      }
    }
    const last = messages[messages.length - 1] as Record<string, unknown> | undefined;
    return last ? String(last['content'] ?? last) : '';
  }

  // ── native invocation ──────────────────────────────────────────────────────

  async invokeNative(
    graph:      unknown,
    _d:         CapabilityDescriptor,
    input:      LangGraphInput,
  ): Promise<LangGraphOutput> {
    const g = graph as Record<string, unknown>;
    const cfg = { recursionLimit: this.lgConfig.recursionLimit };

    if (this.lgConfig.stream && typeof g['stream'] === 'function') {
      const chunks: LangGraphOutput = {};
      for await (const chunk of (g['stream'] as Function)(input, cfg)) {
        if (typeof chunk === 'object') Object.assign(chunks, chunk);
      }
      return chunks;
    }

    if (typeof g['invoke'] === 'function') {
      return (g['invoke'] as Function)(input, cfg);
    }
    if (typeof g['ainvoke'] === 'function') {
      return (g['ainvoke'] as Function)(input, cfg);
    }

    throw new Error('Graph does not have invoke() or ainvoke() method');
  }
}

// ── one-liner helper ──────────────────────────────────────────────────────────

export function wrapLangGraph(
  graph:    unknown,
  config:   LangGraphPluginConfig,
  tools?:   unknown[],
) {
  return new LangGraphPlugin(config, tools).wrap(graph);
}
