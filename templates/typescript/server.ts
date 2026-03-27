/**
 * Borgkit HTTP Server (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Starts an Express HTTP server for a WrappedAgent, exposing the standard
 * Borgkit endpoints so agents can discover and call each other over the network.
 *
 * Endpoints
 * ─────────
 *   POST /invoke         AgentRequest  → AgentResponse (JSON)
 *   POST /invoke/stream  AgentRequest  → Server-Sent Events (StreamChunk / StreamEnd)
 *   POST /gossip         GossipMessage (direct fan-out from peers)
 *   GET  /health         Heartbeat — lightweight, no auth
 *   GET  /anr            Full ANR (Agent Network Record) as JSON
 *   GET  /capabilities   Capability list as JSON
 *
 * Usage
 * ─────
 *   import { serve } from './server';
 *
 *   const plugin = new LangGraphPlugin(config);
 *   const agent  = plugin.wrap(myGraph);
 *   await serve(agent, { port: 6174 });
 *
 *   // or via borgkit-cli:
 *   //   borgkit run MyAgent --port 6174
 */

import http            from 'http';
import express, { Request, Response, NextFunction } from 'express';
import type { IAgent } from './interfaces/IAgent';
import type { AgentRequest }  from './interfaces/IAgentRequest';

// ── types ──────────────────────────────────────────────────────────────────────

export interface ServeOptions {
  /** Bind address. Default: '0.0.0.0' (all interfaces). */
  host?: string;
  /** TCP port. Overridden by BORGKIT_PORT env var if set. Default: 6174. */
  port?: number;
  /** Suppress the startup banner log. Default: false. */
  silent?: boolean;
}

// ── CORS headers ───────────────────────────────────────────────────────────────

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin':  '*',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization, X-Payment, X-402-Payment',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
};

// ── public entry point ─────────────────────────────────────────────────────────

/**
 * Start the HTTP transport for *agent* and resolve when the server is ready.
 *
 * The server registers with the discovery layer *after* it starts listening,
 * so peers can connect immediately after the ANR is announced on the mesh.
 *
 * Graceful shutdown is wired to SIGINT and SIGTERM automatically.
 */
export async function serve(
  agent:   IAgent,
  options: ServeOptions = {},
): Promise<void> {
  const host = options.host ?? '0.0.0.0';
  const port = parseInt(process.env.BORGKIT_PORT ?? String(options.port ?? 6174), 10);

  const app = express();
  app.use(express.json({ limit: '4mb' }));

  // ── CORS pre-flight ──────────────────────────────────────────────────────────
  app.options('*', (_req: Request, res: Response) => {
    Object.entries(CORS_HEADERS).forEach(([k, v]) => res.setHeader(k, v));
    res.sendStatus(204);
  });

  function cors(_req: Request, res: Response, next: NextFunction): void {
    Object.entries(CORS_HEADERS).forEach(([k, v]) => res.setHeader(k, v));
    next();
  }

  // ── POST /invoke ─────────────────────────────────────────────────────────────
  app.post('/invoke', cors, async (req: Request, res: Response): Promise<void> => {
    const body = req.body ?? {};

    if (!body.capability) {
      res.status(400).json({ error: 'Missing required field: capability' });
      return;
    }

    // x402 gate — check before dispatching if the agent has pricing
    const x402Challenge = checkX402(agent, body);
    if (x402Challenge) {
      res.status(402).json(x402Challenge);
      return;
    }

    const agentReq: AgentRequest = {
      requestId:  body.requestId  ?? crypto.randomUUID(),
      from:       body.from       ?? 'anonymous',
      capability: body.capability,
      payload:    body.payload    ?? {},
      signature:  body.signature,
      timestamp:  body.timestamp  ?? Date.now(),
      x402:       body.x402,
    };

    try {
      const agentRes = await agent.handleRequest(agentReq);
      res.json(agentRes);
    } catch (err) {
      res.status(500).json({
        requestId:    agentReq.requestId,
        status:       'error',
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  });

  // ── POST /invoke/stream ──────────────────────────────────────────────────────
  app.post('/invoke/stream', cors, async (req: Request, res: Response): Promise<void> => {
    const body = req.body ?? {};

    if (!body.capability) {
      res.status(400).json({ error: 'Missing required field: capability' });
      return;
    }

    // x402 gate — same logic as /invoke
    const x402Challenge = checkX402(agent, body);
    if (x402Challenge) {
      // Signal the payment requirement as an SSE error frame then close
      res.setHeader('Content-Type', 'text/event-stream');
      res.setHeader('Cache-Control', 'no-cache');
      res.setHeader('X-Accel-Buffering', 'no');
      res.status(402).write(`data: ${JSON.stringify({ type: 'error', error: 'Payment required', x402: x402Challenge })}\n\n`);
      res.end();
      return;
    }

    const agentReq: AgentRequest = {
      requestId:  body.requestId  ?? crypto.randomUUID(),
      from:       body.from       ?? 'anonymous',
      capability: body.capability,
      payload:    body.payload    ?? {},
      signature:  body.signature,
      timestamp:  body.timestamp  ?? Date.now(),
      x402:       body.x402,
      stream:     true,
    };

    // Set up SSE headers
    res.setHeader('Content-Type',      'text/event-stream');
    res.setHeader('Cache-Control',     'no-cache');
    res.setHeader('X-Accel-Buffering', 'no');   // nginx: disable proxy buffering
    res.flushHeaders();

    try {
      const streamer = (agent as any).streamRequest;
      if (typeof streamer !== 'function') {
        // Agent does not support streaming — fall back to single-shot
        const agentRes = await agent.handleRequest(agentReq);
        const content  = (agentRes.result as any)?.content ?? '';
        if (content) {
          res.write(`data: ${JSON.stringify({ type: 'chunk', requestId: agentReq.requestId, delta: String(content), result: agentRes.result, sequence: 0, timestamp: Date.now() })}\n\n`);
        }
        res.write(`data: ${JSON.stringify({ type: 'end', requestId: agentReq.requestId, finalResult: agentRes.result, sequence: content ? 1 : 0, timestamp: Date.now() })}\n\n`);
        res.end();
        return;
      }

      for await (const event of (agent as any).streamRequest(agentReq)) {
        res.write(`data: ${JSON.stringify(event)}\n\n`);
        if (event.type === 'end') break;
      }
      res.end();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      try {
        res.write(`data: ${JSON.stringify({ type: 'end', requestId: agentReq.requestId, error: msg, sequence: 0, timestamp: Date.now() })}\n\n`);
        res.end();
      } catch { /* client disconnected */ }
    }
  });

  // ── POST /gossip ─────────────────────────────────────────────────────────────
  app.post('/gossip', cors, async (req: Request, res: Response): Promise<void> => {
    try {
      if (agent.handleGossip) {
        await agent.handleGossip(req.body);
      }
    } catch {
      // gossip is best-effort — never crash the server
    }
    res.sendStatus(204);
  });

  // ── GET /health ──────────────────────────────────────────────────────────────
  app.get('/health', cors, async (req: Request, res: Response): Promise<void> => {
    try {
      const nonce = (req.query.nonce as string) ?? String(Date.now());
      if (agent.handleHeartbeat) {
        const hbRes = await agent.handleHeartbeat({ senderId: '__health__', nonce });
        res.json(hbRes);
        return;
      }
      // fallback minimal response
      res.json({
        agentId:           agent.agentId,
        status:            'healthy',
        capabilitiesCount: agent.getCapabilities().length,
        timestamp:         Date.now(),
      });
    } catch (err) {
      res.status(500).json({ error: String(err) });
    }
  });

  // ── GET /anr ─────────────────────────────────────────────────────────────────
  app.get('/anr', cors, (_req: Request, res: Response): void => {
    try {
      res.json(agent.getAnr());
    } catch (err) {
      res.status(500).json({ error: String(err) });
    }
  });

  // ── GET /capabilities ────────────────────────────────────────────────────────
  app.get('/capabilities', cors, (_req: Request, res: Response): void => {
    res.json({
      agentId:      agent.agentId,
      capabilities: agent.getCapabilities(),
    });
  });

  // ── start server ──────────────────────────────────────────────────────────────
  await new Promise<void>((resolve, reject) => {
    const server = http.createServer(app);

    server.listen(port, host, () => resolve());
    server.once('error', reject);

    // ── graceful shutdown ──────────────────────────────────────────────────────
    const shutdown = async (signal: string): Promise<void> => {
      if (!options.silent) {
        process.stdout.write(`\n[Borgkit] ${signal} received — shutting down gracefully…\n`);
      }
      server.close(async () => {
        try {
          if (agent.unregisterDiscovery) await agent.unregisterDiscovery();
        } catch { /* best-effort */ }
        process.exit(0);
      });
    };

    process.once('SIGINT',  () => shutdown('SIGINT'));
    process.once('SIGTERM', () => shutdown('SIGTERM'));
  });

  // Register with discovery *after* the port is bound
  if (agent.registerDiscovery) {
    await agent.registerDiscovery();
  }

  // Block forever (server is listening in background via libuv)
  await new Promise<void>(() => { /* resolved only by shutdown() */ });
}

// ── x402 helper ───────────────────────────────────────────────────────────────

function checkX402(agent: IAgent, body: Record<string, unknown>): Record<string, unknown> | null {
  const capability = body.capability as string;

  // Access x402Pricing via the plugin config if available
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const pricing = (agent as any)?.plugin?.config?.x402Pricing?.[capability]
                ?? (agent as any)?.config?.x402Pricing?.[capability];

  if (!pricing) return null;          // capability is free
  if (body.x402)  return null;        // payment proof already attached

  const network = pricing.network     ?? 'base';
  const amount  = pricing.amountUsd   ?? 0;
  const payee   = pricing.payeeAddress ?? '';

  return {
    error:      'Payment required',
    x402:       true,
    capability,
    price_usd:  amount,
    network,
    payee,
    accepts: [
      {
        scheme:             'exact',
        network,
        maxAmountRequired:  String(Math.round(amount * 1_000_000)),  // USDC 6-decimals
        resource:           `/invoke`,
        asset:              '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913',  // USDC on Base
      },
    ],
  };
}
