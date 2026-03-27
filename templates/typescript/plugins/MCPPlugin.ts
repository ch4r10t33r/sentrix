/**
 * Borgkit ↔ MCP Bridge — TypeScript (inbound direction)
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps any MCP-compatible server as a Borgkit agent.
 *
 * Every MCP tool exposed by the server becomes a Borgkit capability.
 * The wrapped agent can be registered with discovery, called by other Borgkit
 * agents, and served over HTTP — without changing the underlying MCP server.
 *
 * Supported transports
 * ────────────────────
 *   • Stdio  — MCP server launched as a subprocess (most common)
 *   • SSE    — MCP server reachable over HTTP Server-Sent Events
 *   • HTTP   — Streamable HTTP (MCP spec 2025-03-26+)
 *
 * Usage
 * ─────
 * ```ts
 * import { MCPPlugin } from './plugins/MCPPlugin';
 *
 * // Subprocess MCP server
 * const plugin = await MCPPlugin.fromCommand(
 *   ['npx', '-y', '@modelcontextprotocol/server-github'],
 *   config,
 *   { GITHUB_TOKEN: 'ghp_...' },
 * );
 *
 * // SSE / HTTP MCP server
 * const plugin = await MCPPlugin.fromUrl('http://localhost:3000/sse', config);
 *
 * const agent = plugin.wrap();           // IAgent
 * await agent.serve({ port: 8081 });
 * ```
 */

import { BorgkitPlugin, WrappedAgent, PluginConfig, CapabilityDescriptor } from './IPlugin';
import { AgentRequest }  from '../interfaces/IAgentRequest';
import { AgentResponse } from '../interfaces/IAgentResponse';

// ── internal tool representation ───────────────────────────────────────────────

interface MCPTool {
  name:        string;
  description: string;
  inputSchema: Record<string, unknown> | null;
}

// ── MCPPlugin ─────────────────────────────────────────────────────────────────

/**
 * Borgkit plugin that wraps any MCP server as a discoverable Borgkit agent.
 *
 * Build instances with the static async factory methods — do not use `new`
 * directly (the MCP client session must be established asynchronously).
 */
export class MCPPlugin extends BorgkitPlugin<null, Record<string, unknown>, unknown> {
  private client:  unknown = null;   // @modelcontextprotocol/sdk Client
  private tools:   MCPTool[] = [];
  private cleanup: (() => Promise<void>) | null = null;

  constructor(config: PluginConfig) {
    super(config);
  }

  // ── factory methods ─────────────────────────────────────────────────────────

  /**
   * Launch `command` as a subprocess MCP server and connect to it.
   *
   * @param command  argv array, e.g. `['npx', '-y', '@modelcontextprotocol/server-github']`
   * @param config   Borgkit PluginConfig for the resulting agent.
   * @param env      Extra env vars forwarded to the subprocess.
   *
   * @example
   * ```ts
   * const plugin = await MCPPlugin.fromCommand(
   *   ['npx', '-y', '@modelcontextprotocol/server-filesystem', '/tmp'],
   *   config,
   * );
   * ```
   */
  static async fromCommand(
    command: string[],
    config:  PluginConfig,
    env?:    Record<string, string>,
  ): Promise<MCPPlugin> {
    const { Client }              = await import('@modelcontextprotocol/sdk/client/index.js');
    const { StdioClientTransport } = await import('@modelcontextprotocol/sdk/client/stdio.js');

    const plugin    = new MCPPlugin(config);
    const transport = new StdioClientTransport({
      command: command[0],
      args:    command.slice(1),
      env:     { ...process.env, ...env } as Record<string, string>,
    });

    const client = new Client(
      { name: config.name, version: config.version ?? '1.0.0' },
      { capabilities: {} },
    );
    await client.connect(transport);

    plugin.client  = client;
    plugin.cleanup = () => client.close();
    await plugin.refreshTools();
    return plugin;
  }

  /**
   * Connect to an MCP server over SSE or Streamable HTTP.
   *
   * @param url      SSE endpoint (`http://host/sse`) or Streamable HTTP (`http://host/mcp`)
   * @param config   Borgkit PluginConfig.
   * @param headers  Optional HTTP headers (e.g. `{ Authorization: 'Bearer sk-...' }`).
   *
   * @example
   * ```ts
   * const plugin = await MCPPlugin.fromUrl(
   *   'http://localhost:3000/sse',
   *   config,
   *   { Authorization: 'Bearer sk-...' },
   * );
   * ```
   */
  static async fromUrl(
    url:     string,
    config:  PluginConfig,
    headers?: Record<string, string>,
  ): Promise<MCPPlugin> {
    const { Client } = await import('@modelcontextprotocol/sdk/client/index.js');

    const plugin = new MCPPlugin(config);
    const transport = await pickHttpTransport(url, headers);

    const client = new Client(
      { name: config.name, version: config.version ?? '1.0.0' },
      { capabilities: {} },
    );
    await client.connect(transport);

    plugin.client  = client;
    plugin.cleanup = () => client.close();
    await plugin.refreshTools();
    return plugin;
  }

  // ── lifecycle ────────────────────────────────────────────────────────────────

  /** Disconnect from the MCP server and release resources. */
  async close(): Promise<void> {
    if (this.cleanup) {
      await this.cleanup();
      this.cleanup = null;
      this.client  = null;
    }
  }

  /** Re-fetch the tool list from the live MCP server. */
  async refreshTools(): Promise<void> {
    if (!this.client) return;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const result  = await (this.client as any).listTools();
    this.tools = (result.tools ?? []).map((t: Record<string, unknown>) => ({
      name:        t.name as string,
      description: (t.description as string) ?? '',
      inputSchema: (typeof t.inputSchema === 'object' ? t.inputSchema : null) as Record<string, unknown> | null,
    }));
  }

  // ── BorgkitPlugin contract ───────────────────────────────────────────────────

  extractCapabilities(_agent: null): CapabilityDescriptor[] {
    return this.tools.map(t => ({
      name:        t.name,
      description: t.description,
      inputSchema: t.inputSchema ?? undefined,
      nativeName:  t.name,
    }));
  }

  translateRequest(req: AgentRequest, _d: CapabilityDescriptor): Record<string, unknown> {
    return req.payload as Record<string, unknown>;
  }

  translateResponse(nativeResult: unknown, requestId: string): AgentResponse {
    const items = Array.isArray(nativeResult) ? nativeResult : [nativeResult];
    const parts: string[] = [];
    const blobs: unknown[] = [];

    for (const item of items) {
      const rec = item as Record<string, unknown>;
      if (rec?.type === 'text') {
        parts.push(String(rec.text ?? ''));
      } else if (rec?.type === 'image') {
        blobs.push({ type: 'image', mimeType: rec.mimeType, data: rec.data });
      } else if (rec?.type === 'resource') {
        const res = rec.resource as Record<string, unknown> | undefined;
        parts.push(`[resource: ${res?.uri ?? JSON.stringify(rec)}]`);
      } else if (item != null) {
        parts.push(JSON.stringify(item));
      }
    }

    const result: Record<string, unknown> = {};
    if (parts.length)  result['text']  = parts.join('\n');
    if (blobs.length)  result['blobs'] = blobs;
    if (!parts.length && !blobs.length) result['raw'] = JSON.stringify(nativeResult);

    return { requestId, status: 'success', result, timestamp: Date.now() };
  }

  async invokeNative(
    _agent:      null,
    descriptor:  CapabilityDescriptor,
    nativeInput: Record<string, unknown>,
  ): Promise<unknown> {
    if (!this.client) {
      throw new Error('MCPPlugin: client not connected. Use fromCommand() or fromUrl() first.');
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const result = await (this.client as any).callTool({
      name:      descriptor.nativeName,
      arguments: nativeInput,
    });
    return result.content;
  }

  /** Override wrap() so the internal null-agent pattern works cleanly. */
  override wrap(_agent: null = null): WrappedAgent<null, Record<string, unknown>, unknown> {
    const caps = this.extractCapabilities(null);
    return new WrappedAgent(null, this, caps, this.config);
  }
}

// ── transport picker ──────────────────────────────────────────────────────────

async function pickHttpTransport(url: string, headers?: Record<string, string>) {
  // Try Streamable HTTP (MCP spec 2025-03-26+) first
  try {
    const { StreamableHTTPClientTransport } = await import(
      '@modelcontextprotocol/sdk/client/streamableHttp.js'
    );
    return new StreamableHTTPClientTransport(new URL(url), {
      requestInit: headers ? { headers } : undefined,
    });
  } catch { /* not available in this SDK version */ }

  // Fall back to SSE transport
  try {
    const { SSEClientTransport } = await import('@modelcontextprotocol/sdk/client/sse.js');
    return new SSEClientTransport(new URL(url), {
      requestInit: headers ? { headers } : undefined,
    });
  } catch { /* not available */ }

  throw new Error(
    'MCPPlugin: no HTTP transport available.\n' +
    'Upgrade: npm install @modelcontextprotocol/sdk@latest',
  );
}
