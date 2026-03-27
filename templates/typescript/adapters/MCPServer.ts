/**
 * Borgkit ↔ MCP Bridge — TypeScript (outbound direction)
 * ─────────────────────────────────────────────────────────────────────────────
 * Exposes any Borgkit agent (IAgent) as an MCP server.
 *
 * Every Borgkit capability becomes an MCP tool that any MCP-compatible
 * client (Claude Desktop, Cursor, Continue, etc.) can call without knowing
 * anything about the Borgkit protocol.
 *
 * Transports
 * ──────────
 *   stdio  (default) — connect via MCP host config file
 *   sse              — HTTP Server-Sent Events, useful for remote access
 *   http             — Streamable HTTP (MCP spec 2025-03-26+)
 *
 * Usage — stdio (Claude Desktop / Cursor)
 * ────────────────────────────────────────
 * ```ts
 * import { serveAsMcp } from './adapters/MCPServer';
 * await serveAsMcp(myAgent);
 * ```
 *
 * Claude Desktop config  (~/.config/claude/claude_desktop_config.json):
 * ```json
 * {
 *   "mcpServers": {
 *     "my-agent": {
 *       "command": "npx",
 *       "args": ["ts-node", "run_mcp.ts"]
 *     }
 *   }
 * }
 * ```
 *
 * Usage — SSE (remote access)
 * ───────────────────────────
 * ```ts
 * await serveAsMcp(agent, { transport: 'sse', port: 3000 });
 * // clients connect to http://localhost:3000/sse
 * ```
 */

import { IAgent } from '../interfaces/IAgent';
import type { AgentRequest } from '../interfaces/IAgentRequest';

export type TransportMode = 'stdio' | 'sse' | 'http';

export interface ServeMcpOptions {
  /** MCP server name (defaults to agent.agentId). */
  name?:      string;
  /** Transport to use. Default: 'stdio'. */
  transport?: TransportMode;
  /** Bind address for SSE / HTTP transports. Default: '0.0.0.0'. */
  host?:      string;
  /** TCP port for SSE / HTTP transports. Default: 3000. */
  port?:      number;
}

// ── public entry point ─────────────────────────────────────────────────────────

/**
 * Expose *agent* as an MCP server.
 *
 * Blocks until the transport closes (stdio EOF) or SIGINT / SIGTERM
 * is received (network transports).
 *
 * @param agent    Any Borgkit IAgent instance.
 * @param options  Transport and binding options.
 */
export async function serveAsMcp(
  agent:   IAgent,
  options: ServeMcpOptions = {},
): Promise<void> {
  const { Server }                  = await import('@modelcontextprotocol/sdk/server/index.js');
  const { ListToolsRequestSchema, CallToolRequestSchema } =
    await import('@modelcontextprotocol/sdk/types.js');

  const serverName    = options.name ?? agent.agentId;
  const serverVersion = agentVersion(agent);
  const transport     = options.transport ?? 'stdio';
  const host          = options.host ?? '0.0.0.0';
  const port          = options.port ?? 3000;

  const server = new Server(
    { name: serverName, version: serverVersion },
    { capabilities: { tools: {} } },
  );

  // ── list tools ──────────────────────────────────────────────────────────────

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: agent.getCapabilities()
      .filter(cap => !cap.startsWith('__'))   // hide reserved mesh capabilities
      .map(cap => ({
        name:        cap,
        description: capDescription(agent, cap),
        inputSchema: capSchema(agent, cap),
      })),
  }));

  // ── call tool ───────────────────────────────────────────────────────────────

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const args = request.params.arguments as Record<string, unknown> | undefined ?? {};

    const req: AgentRequest = {
      requestId:  crypto.randomUUID(),
      from:       'mcp_client',
      capability: request.params.name,
      // Accept both { payload: {...} } and flat { key: value } forms
      payload:    (args['payload'] as Record<string, unknown>) ?? args,
      timestamp:  Date.now(),
    };

    const resp = await agent.handleRequest(req);

    let text: string;
    if (resp.status === 'success') {
      text = resp.result != null
        ? JSON.stringify(resp.result, null, 2)
        : 'OK';
    } else if (resp.status === 'payment_required') {
      text = `Payment required to call '${req.capability}'.\n` +
             `Requirements: ${JSON.stringify(resp.paymentRequirements)}`;
    } else {
      text = `Error: ${resp.errorMessage}`;
    }

    return { content: [{ type: 'text' as const, text }] };
  });

  // ── start transport ─────────────────────────────────────────────────────────

  if (transport === 'stdio') {
    await runStdio(server);
  } else if (transport === 'sse') {
    await runSse(server, host, port, serverName);
  } else if (transport === 'http') {
    await runHttp(server, host, port, serverName);
  } else {
    throw new Error(`Unknown transport: "${transport}". Use 'stdio', 'sse', or 'http'.`);
  }
}

// ── transport runners ──────────────────────────────────────────────────────────

async function runStdio(server: unknown): Promise<void> {
  const { StdioServerTransport } =
    await import('@modelcontextprotocol/sdk/server/stdio.js');
  const xport = new StdioServerTransport();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  await (server as any).connect(xport);
  // Keep alive until stdin closes
  await new Promise<void>((resolve) => {
    process.stdin.once('close', resolve);
    process.once('SIGINT',  resolve);
    process.once('SIGTERM', resolve);
  });
}

async function runSse(
  server:     unknown,
  host:       string,
  port:       number,
  serverName: string,
): Promise<void> {
  const { SSEServerTransport } =
    await import('@modelcontextprotocol/sdk/server/sse.js');
  const express = (await import('express')).default;
  const app     = express();

  const connections = new Map<string, unknown>();

  app.get('/sse', async (_req, res) => {
    const xport = new SSEServerTransport('/messages', res);
    connections.set(xport.sessionId, xport);
    res.on('close', () => connections.delete(xport.sessionId));
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    await (server as any).connect(xport);
  });

  app.post('/messages', express.json(), async (req, res) => {
    const sessionId = req.query.sessionId as string;
    const xport     = connections.get(sessionId) as { handlePostMessage?: Function } | undefined;
    if (xport?.handlePostMessage) {
      await xport.handlePostMessage(req, res);
    } else {
      res.status(404).json({ error: 'Session not found' });
    }
  });

  await new Promise<void>((resolve, reject) => {
    app.listen(port, host, () => {
      console.log(`[Borgkit→MCP] SSE server '${serverName}' on http://${host}:${port}/sse`);
      resolve();
    }).once('error', reject);
  });

  // Block until shutdown
  await new Promise<void>((resolve) => {
    process.once('SIGINT',  resolve);
    process.once('SIGTERM', resolve);
  });
}

async function runHttp(
  server:     unknown,
  host:       string,
  port:       number,
  serverName: string,
): Promise<void> {
  const { StreamableHTTPServerTransport } =
    await import('@modelcontextprotocol/sdk/server/streamableHttp.js');
  const express = (await import('express')).default;
  const app     = express();
  app.use(express.json());

  app.all('/mcp', async (req, res) => {
    const xport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => crypto.randomUUID(),
    });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    await (server as any).connect(xport);
    await xport.handleRequest(req, res);
  });

  await new Promise<void>((resolve, reject) => {
    app.listen(port, host, () => {
      console.log(`[Borgkit→MCP] HTTP server '${serverName}' on http://${host}:${port}/mcp`);
      resolve();
    }).once('error', reject);
  });

  await new Promise<void>((resolve) => {
    process.once('SIGINT',  resolve);
    process.once('SIGTERM', resolve);
  });
}

// ── helpers ────────────────────────────────────────────────────────────────────

function agentVersion(agent: IAgent): string {
  const meta = agent.metadata;
  if (meta && typeof meta === 'object' && 'version' in meta) {
    return String((meta as Record<string, unknown>).version ?? '0.1.0');
  }
  return '0.1.0';
}

function capDescription(agent: IAgent, cap: string): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const caps = (agent as any).capMap as Map<string, { description?: string }> | undefined;
    const desc = caps?.get(cap)?.description;
    if (desc) return desc;
  } catch { /* no capMap */ }
  return `Borgkit capability: ${cap}`;
}

function capSchema(agent: IAgent, cap: string): Record<string, unknown> {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const caps = (agent as any).capMap as Map<string, { inputSchema?: Record<string, unknown> }> | undefined;
    const schema = caps?.get(cap)?.inputSchema;
    if (schema) return schema;
  } catch { /* no capMap */ }
  return {
    type: 'object',
    properties: {
      payload: {
        type:        'object',
        description: `Input payload for the '${cap}' capability.`,
      },
    },
  };
}
