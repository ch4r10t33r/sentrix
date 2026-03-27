/**
 * Libp2PListener — registers libp2p stream handlers so a Borgkit IAgent
 * can receive AgentRequests over a raw P2P stream without an HTTP server.
 *
 * Protocols handled:
 *   /borgkit/invoke/1.0.0   — single request → single response (LP-framed JSON)
 *   /borgkit/gossip/1.0.0   — fire-and-forget gossip message (LP-framed JSON)
 *
 * LP framing: each message is prefixed with a 4-byte big-endian uint32 length.
 */
import type { Libp2p, Stream } from 'libp2p';
import type { IAgent }         from '../../interfaces/IAgent';
import { AgentRequest }        from '../../interfaces/IAgentRequest';

export const INVOKE_PROTO  = '/borgkit/invoke/1.0.0';
export const GOSSIP_PROTO  = '/borgkit/gossip/1.0.0';
export const STREAM_PROTO  = '/borgkit/stream/1.0.0';

export class Libp2PListener {
  constructor(
    private readonly node:  Libp2p,
    private readonly agent: IAgent,
  ) {}

  /** Register all Borgkit protocol handlers on the node. */
  register(): void {
    this.node.handle(INVOKE_PROTO, ({ stream }) => this._handleInvoke(stream).catch(() => {}));
    this.node.handle(GOSSIP_PROTO, ({ stream }) => this._handleGossip(stream).catch(() => {}));
  }

  /** Unregister handlers (call before stopping the node). */
  async unregister(): Promise<void> {
    await this.node.unhandle(INVOKE_PROTO);
    await this.node.unhandle(GOSSIP_PROTO);
  }

  // ── invoke handler ──────────────────────────────────────────────────────────

  private async _handleInvoke(stream: Stream): Promise<void> {
    try {
      const data = await readLPFrame(stream);
      const raw  = JSON.parse(data.toString('utf8')) as Record<string, unknown>;
      const req  = AgentRequest.fromDict(raw);
      const resp = await this.agent.handleRequest(req);
      await writeLPFrame(stream, Buffer.from(JSON.stringify(resp), 'utf8'));
    } finally {
      stream.close().catch(() => {});
    }
  }

  // ── gossip handler ──────────────────────────────────────────────────────────

  private async _handleGossip(stream: Stream): Promise<void> {
    try {
      const data = await readLPFrame(stream);
      const msg  = JSON.parse(data.toString('utf8'));
      // Deliver to the agent's gossip handler (reserved capability __gossip)
      const fakeReq = new AgentRequest({
        requestId:  crypto.randomUUID(),
        fromId:     msg.senderId ?? 'unknown',
        capability: '__gossip',
        payload:    msg,
        timestamp:  Date.now(),
      });
      await this.agent.handleRequest(fakeReq).catch(() => {});
    } finally {
      stream.close().catch(() => {});
    }
  }
}

// ── LP framing helpers ─────────────────────────────────────────────────────────

/** Read one LP-framed message: 4-byte big-endian length + payload. */
export async function readLPFrame(stream: Stream): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let   total = 0;

  for await (const chunk of stream.source) {
    chunks.push(Buffer.from(chunk instanceof Uint8Array ? chunk : chunk.subarray()));
    total += chunk.byteLength;
    // Try to parse as soon as we have enough bytes
    const buf = Buffer.concat(chunks, total);
    if (buf.length >= 4) {
      const len = buf.readUInt32BE(0);
      if (buf.length >= 4 + len) {
        return buf.subarray(4, 4 + len);
      }
    }
  }
  throw new Error('Stream ended before LP frame was complete');
}

/** Write one LP-framed message: 4-byte big-endian length + payload. */
export async function writeLPFrame(stream: Stream, payload: Buffer): Promise<void> {
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32BE(payload.length, 0);
  const frame  = Buffer.concat([header, payload]);
  await stream.sink([frame]);
}
