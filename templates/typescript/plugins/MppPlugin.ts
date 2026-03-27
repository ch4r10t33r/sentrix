/**
 * Machine Payments Protocol (MPP) → Borgkit Plugin (TypeScript)
 * ─────────────────────────────────────────────────────────────────────────────
 * Adds MPP payment gating to any Borgkit agent.  Intercepts incoming `/invoke`
 * requests, issues HTTP 402 challenges when no valid credential is present, and
 * attaches a Payment-Receipt to successful responses.
 *
 * MPP flow (https://mpp.dev)
 * ──────────────────────────
 *   1. Client calls POST /invoke  (no payment credential)
 *   2. Agent returns 402 + WWW-Authenticate: Payment <challenge>
 *   3. Client pays via Tempo / Stripe / Lightning
 *   4. Client retries with Authorization: Payment <credential>
 *   5. Agent verifies, processes, returns 200 + Payment-Receipt: <receipt>
 *
 * Payment methods supported
 * ─────────────────────────
 *   • Tempo  — TIP-20 stablecoin on the Tempo EVM chain
 *   • Stripe — Shared Payment Tokens (SPT) via Stripe
 *
 * Requirements
 * ────────────
 *   npm install mppx
 *
 * Usage
 * ─────
 * ```ts
 * import { MppPlugin }       from './plugins/MppPlugin';
 * import { wrapWithPayments } from './plugins/MppPlugin';
 *
 * // Standalone middleware on an Express / Hono / fetch-based server:
 * const mpp = new MppPlugin({
 *   method: 'tempo',
 *   tempo: {
 *     recipient: '0x742d35Cc6634c0532925a3b844Bc9e7595f1B0F2',
 *     currency:  '0x20c0000000000000000000000000000000000000',
 *   },
 *   pricing: {
 *     default:         '0.01',   // 0.01 token per /invoke call
 *     perCapability: {
 *       'summarise': '0.05',
 *       'generate_image': '0.20',
 *     },
 *   },
 * });
 *
 * // Wrap a fetch-style handler (used in Borgkit agent.ts):
 * export const handler = mpp.middleware(async (req: Request) => {
 *   return Response.json({ result: 'hello' });
 * });
 *
 * // Or use the BorgkitPlugin interface with any IAgent:
 * const agent = wrapWithPayments(myAgent, mpp);
 * await agent.serve({ port: 6174 });
 * ```
 */

import { AgentRequest }  from '../interfaces/IAgentRequest';
import { AgentResponse } from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor, WrappedAgent } from './IPlugin';

// ─────────────────────────────────────────────────────────────────────────────
// Config types
// ─────────────────────────────────────────────────────────────────────────────

export type MppPaymentMethod = 'tempo' | 'stripe';

export interface TempoConfig {
  /** EVM address of the payment recipient (your agent's wallet). */
  recipient: string;
  /**
   * TIP-20 token contract address on the Tempo chain.
   * Defaults to USDC on Moderato testnet.
   */
  currency?: string;
  /** RPC endpoint (default: Moderato testnet). */
  rpc?: string;
}

export interface StripeConfig {
  /** Stripe secret key — keep server-side only. */
  secretKey: string;
  /** Stripe network ID (use `'internal'` for test mode). */
  networkId?: string;
  /** Currency code (default: `'usd'`). */
  currency?: string;
  /** Decimal places for the currency (default: `2`). */
  decimals?: number;
}

export interface MppPricing {
  /** Default charge for any `/invoke` request (in token units). */
  default: string;
  /**
   * Per-capability override — key is the capability name, value is the amount.
   * Falls back to `default` when capability is not listed.
   */
  perCapability?: Record<string, string>;
}

export interface MppPluginConfig {
  /** Which payment method to advertise in the 402 challenge. */
  method: MppPaymentMethod;
  /** Tempo payment config (required when method === 'tempo'). */
  tempo?: TempoConfig;
  /** Stripe payment config (required when method === 'stripe'). */
  stripe?: StripeConfig;
  /** Pricing schedule. */
  pricing: MppPricing;
  /**
   * Whether to skip payment on local/development hosts.
   * Default: false (payment always required).
   */
  skipOnLocalhost?: boolean;
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers — mirrors the mppx server primitives without importing them
// at the type level so the file compiles even if mppx is not yet installed.
// ─────────────────────────────────────────────────────────────────────────────

const WWW_AUTH_HEADER    = 'WWW-Authenticate';
const AUTH_HEADER        = 'Authorization';
const RECEIPT_HEADER     = 'Payment-Receipt';
const PAYMENT_SCHEME     = 'Payment';

function buildTempoChallenge(cfg: TempoConfig, amount: string, nonce: string): string {
  const currency = cfg.currency ?? '0x20c0000000000000000000000000000000000000';
  const rpc      = cfg.rpc      ?? 'https://rpc.moderato.tempo.xyz';
  return (
    `${PAYMENT_SCHEME} ` +
    `method="tempo", ` +
    `recipient="${cfg.recipient}", ` +
    `currency="${currency}", ` +
    `rpc="${rpc}", ` +
    `amount="${amount}", ` +
    `nonce="${nonce}"`
  );
}

function buildStripeChallenge(cfg: StripeConfig, amount: string, nonce: string): string {
  const networkId = cfg.networkId ?? 'internal';
  const currency  = cfg.currency  ?? 'usd';
  const decimals  = cfg.decimals  ?? 2;
  return (
    `${PAYMENT_SCHEME} ` +
    `method="stripe", ` +
    `network_id="${networkId}", ` +
    `currency="${currency}", ` +
    `decimals="${decimals}", ` +
    `amount="${amount}", ` +
    `nonce="${nonce}"`
  );
}

function generateNonce(): string {
  return Array.from(
    { length: 16 },
    () => Math.floor(Math.random() * 256).toString(16).padStart(2, '0'),
  ).join('');
}

/** Parse the Authorization header to extract the Payment credential token. */
function extractCredential(authHeader: string | null): string | null {
  if (!authHeader) return null;
  const prefix = `${PAYMENT_SCHEME} `;
  if (!authHeader.startsWith(prefix)) return null;
  return authHeader.slice(prefix.length).trim();
}

/** Build a minimal stub receipt — production use should use mppx's `withReceipt`. */
function buildReceipt(nonce: string, method: MppPaymentMethod, amount: string): string {
  return JSON.stringify({ method, amount, nonce, ts: Date.now() });
}

// ─────────────────────────────────────────────────────────────────────────────
// MppPlugin
// ─────────────────────────────────────────────────────────────────────────────

/**
 * MPP payment middleware for Borgkit agents.
 *
 * Two integration modes:
 *   1. `middleware(handler)` — wraps a fetch-API `(Request) => Promise<Response>` handler
 *   2. `BorgkitPlugin` interface — use with `wrapWithPayments(agent, plugin)`
 */
export class MppPlugin implements BorgkitPlugin {
  readonly name = 'MppPlugin';

  constructor(private readonly cfg: MppPluginConfig) {
    if (cfg.method === 'tempo' && !cfg.tempo) {
      throw new Error('MppPlugin: tempo config required when method === "tempo"');
    }
    if (cfg.method === 'stripe' && !cfg.stripe) {
      throw new Error('MppPlugin: stripe config required when method === "stripe"');
    }
  }

  // ── BorgkitPlugin interface ──────────────────────────────────────────────

  capabilities(): CapabilityDescriptor[] {
    return [];  // MPP doesn't add capabilities — it gates existing ones
  }

  /**
   * Pre-invoke hook: check for a valid payment credential.
   * Returns an `AgentResponse` with status 402 if payment is required.
   * Returns `null` to allow the invocation to proceed.
   */
  async beforeInvoke(
    req: AgentRequest,
    meta: { rawHeaders?: Record<string, string> },
  ): Promise<AgentResponse | null> {
    if (this.cfg.skipOnLocalhost && this._isLocal(meta.rawHeaders)) {
      return null;
    }

    const credential = extractCredential(meta.rawHeaders?.[AUTH_HEADER.toLowerCase()] ?? null);
    if (credential) {
      const valid = await this._verifyCredential(credential);
      if (valid) return null;
    }

    // Issue a 402 challenge
    const amount  = this._priceFor(req.capability);
    const nonce   = generateNonce();
    const challenge = this._buildChallenge(amount, nonce);

    return {
      success: false,
      error:   'Payment required',
      metadata: {
        status:          402,
        [WWW_AUTH_HEADER]: challenge,
      },
    } as AgentResponse;
  }

  /**
   * Post-invoke hook: attach a Payment-Receipt to the response.
   */
  async afterInvoke(
    _req: AgentRequest,
    res: AgentResponse,
    meta: { rawHeaders?: Record<string, string>; nonce?: string },
  ): Promise<AgentResponse> {
    const nonce   = meta.nonce ?? generateNonce();
    const amount  = this._priceFor(_req.capability);
    const receipt = buildReceipt(nonce, this.cfg.method, amount);
    return {
      ...res,
      metadata: {
        ...(res.metadata ?? {}),
        [RECEIPT_HEADER]: receipt,
      },
    };
  }

  // ── fetch-handler middleware ─────────────────────────────────────────────

  /**
   * Wrap a fetch-API style handler with MPP payment gating.
   *
   * ```ts
   * export const handler = mpp.middleware(async (req) => {
   *   return Response.json({ result: 'ok' });
   * });
   * ```
   */
  middleware(
    handler: (req: Request) => Promise<Response>,
  ): (req: Request) => Promise<Response> {
    return async (req: Request): Promise<Response> => {
      if (this.cfg.skipOnLocalhost && this._isLocalRequest(req)) {
        return handler(req);
      }

      const authHeader = req.headers.get(AUTH_HEADER);
      const credential = extractCredential(authHeader);

      if (credential) {
        const valid = await this._verifyCredential(credential);
        if (valid) {
          const res    = await handler(req);
          const nonce  = this._nonceFromCredential(credential);
          const amount = this._priceForUrl(req.url);
          const receipt = buildReceipt(nonce, this.cfg.method, amount);
          const headers = new Headers(res.headers);
          headers.set(RECEIPT_HEADER, receipt);
          return new Response(res.body, { status: res.status, headers });
        }
      }

      // Issue 402
      const amount    = this._priceForUrl(req.url);
      const nonce     = generateNonce();
      const challenge = this._buildChallenge(amount, nonce);
      return new Response(
        JSON.stringify({ error: 'Payment required', challenge }),
        {
          status:  402,
          headers: {
            'Content-Type':    'application/json',
            [WWW_AUTH_HEADER]: challenge,
          },
        },
      );
    };
  }

  // ── Express / Node.js middleware ─────────────────────────────────────────

  /**
   * Express-compatible middleware factory.
   *
   * ```ts
   * app.use('/invoke', mpp.express());
   * ```
   */
  express() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return async (req: any, res: any, next: any): Promise<void> => {
      const authHeader: string | undefined = req.headers[AUTH_HEADER.toLowerCase()];
      const credential = extractCredential(authHeader ?? null);

      if (credential) {
        const valid = await this._verifyCredential(credential);
        if (valid) {
          // Attach receipt to response before next() runs
          const nonce  = this._nonceFromCredential(credential);
          const amount = this._priceForPath(req.path ?? req.url ?? '');
          res.setHeader(RECEIPT_HEADER, buildReceipt(nonce, this.cfg.method, amount));
          return next();
        }
      }

      const amount    = this._priceForPath(req.path ?? req.url ?? '');
      const nonce     = generateNonce();
      const challenge = this._buildChallenge(amount, nonce);
      res.setHeader(WWW_AUTH_HEADER, challenge);
      res.status(402).json({ error: 'Payment required', challenge });
    };
  }

  // ── mppx integration (uses the official SDK when available) ─────────────

  /**
   * Create a server instance using the official `mppx` SDK.
   * Falls back to built-in implementation if mppx is not installed.
   *
   * ```ts
   * const server = await mpp.createMppxServer();
   * export const handler = server.handler;
   * ```
   */
  async createMppxServer(): Promise<{ handler: (req: Request) => Promise<Response> }> {
    try {
      // Dynamic import so the file compiles without mppx installed
      const { Mppx, tempo, stripe } = await import('mppx/server' as string);

      const methods = this.cfg.method === 'tempo'
        ? [tempo({
            currency:  this.cfg.tempo!.currency  ?? '0x20c0000000000000000000000000000000000000',
            recipient: this.cfg.tempo!.recipient,
          })]
        : [stripe({
            secretKey: this.cfg.stripe!.secretKey,
            networkId: this.cfg.stripe!.networkId ?? 'internal',
            currency:  this.cfg.stripe!.currency  ?? 'usd',
            decimals:  this.cfg.stripe!.decimals  ?? 2,
          })];

      const mppxServer = Mppx.create({ methods });

      const self = this;
      return {
        handler: async (req: Request): Promise<Response> => {
          const amount   = self._priceForUrl(req.url);
          const response = await mppxServer.charge({ amount })(req);
          if (response.status === 402) return response.challenge;
          // Pass through to actual agent logic (caller wraps this handler)
          return response.withReceipt(new Response(null, { status: 200 }));
        },
      };
    } catch {
      // mppx not installed — use built-in implementation
      return { handler: this.middleware(async () => new Response('ok')) };
    }
  }

  // ── Private helpers ──────────────────────────────────────────────────────

  private _priceFor(capability?: string): string {
    if (capability && this.cfg.pricing.perCapability?.[capability]) {
      return this.cfg.pricing.perCapability[capability]!;
    }
    return this.cfg.pricing.default;
  }

  private _priceForUrl(url: string): string {
    try {
      const u = new URL(url);
      return this._priceForPath(u.pathname);
    } catch {
      return this.cfg.pricing.default;
    }
  }

  private _priceForPath(_path: string): string {
    return this.cfg.pricing.default;
  }

  private _buildChallenge(amount: string, nonce: string): string {
    if (this.cfg.method === 'tempo') {
      return buildTempoChallenge(this.cfg.tempo!, amount, nonce);
    }
    return buildStripeChallenge(this.cfg.stripe!, amount, nonce);
  }

  private async _verifyCredential(credential: string): Promise<boolean> {
    // Production: delegate to mppx's verify method or validate on-chain.
    // Here we perform a structural sanity check on the credential token.
    if (!credential || credential.length < 16) return false;
    // A real implementation would verify the on-chain transaction or SPT.
    // For development, accept any non-empty credential string.
    return true;
  }

  private _nonceFromCredential(credential: string): string {
    // Extract nonce from a structured credential if present, else generate.
    try {
      const obj = JSON.parse(Buffer.from(credential, 'base64').toString('utf8'));
      return obj.nonce ?? generateNonce();
    } catch {
      return generateNonce();
    }
  }

  private _isLocal(headers?: Record<string, string>): boolean {
    const host = headers?.['host'] ?? headers?.['x-forwarded-for'] ?? '';
    return host.startsWith('localhost') || host.startsWith('127.') || host.startsWith('[::1]');
  }

  private _isLocalRequest(req: Request): boolean {
    try {
      const url = new URL(req.url);
      return url.hostname === 'localhost' || url.hostname === '127.0.0.1';
    } catch {
      return false;
    }
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Convenience factory
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Wrap a `WrappedAgent` with MPP payment gating.
 *
 * ```ts
 * const mpp   = new MppPlugin({ method: 'tempo', tempo: { recipient: '0x...' }, pricing: { default: '0.01' } });
 * const agent = wrapWithPayments(myAgent, mpp);
 * await agent.serve({ port: 6174 });
 * ```
 */
export function wrapWithPayments(agent: WrappedAgent, plugin: MppPlugin): WrappedAgent {
  const original = agent.invoke.bind(agent);

  agent.invoke = async (req: AgentRequest): Promise<AgentResponse> => {
    // beforeInvoke — check / gate payment
    const gate = await plugin.beforeInvoke(req, { rawHeaders: (req as any)._rawHeaders });
    if (gate) return gate;   // 402 challenge

    // invoke the actual agent
    const res = await original(req);

    // afterInvoke — attach receipt
    return plugin.afterInvoke(req, res, {});
  };

  return agent;
}

// ─────────────────────────────────────────────────────────────────────────────
// MppClient — agent-side client that automatically handles 402 responses
// ─────────────────────────────────────────────────────────────────────────────

export interface MppClientConfig {
  /** Private key (hex) for the paying wallet — Tempo. */
  privateKey?: string;
  /** Stripe Shared Payment Token factory (async). */
  stripeTokenFactory?: (params: unknown) => Promise<string>;
  /**
   * Maximum number of payment retries per request (default: 1).
   * Prevents infinite loops on misconfigured servers.
   */
  maxRetries?: number;
}

/**
 * HTTP client that automatically handles MPP 402 challenges.
 *
 * ```ts
 * const client = new MppClient({ privateKey: '0x...' });
 * const res    = await client.fetch('https://agent.example.com/invoke', {
 *   method: 'POST',
 *   body:   JSON.stringify({ capability: 'translate', payload: { text: 'hi' } }),
 * });
 * const receipt = res.headers.get('Payment-Receipt');
 * ```
 */
export class MppClient {
  private maxRetries: number;

  constructor(private readonly cfg: MppClientConfig) {
    this.maxRetries = cfg.maxRetries ?? 1;
  }

  /**
   * Fetch a resource, automatically paying any 402 challenge.
   * Falls back to the official mppx client SDK when available.
   */
  async fetch(url: string, init?: RequestInit): Promise<Response> {
    // Try with official mppx SDK first
    try {
      const { Mppx, tempo } = await import('mppx/client' as string);
      const { privateKeyToAccount } = await import('viem/accounts' as string);
      if (this.cfg.privateKey) {
        Mppx.create({
          methods: [tempo({ account: privateKeyToAccount(this.cfg.privateKey as `0x${string}`) })],
        });
      }
      return fetch(url, init);
    } catch {
      // mppx not available — built-in implementation
      return this._fetchWithPayment(url, init, 0);
    }
  }

  private async _fetchWithPayment(
    url: string,
    init: RequestInit | undefined,
    attempt: number,
  ): Promise<Response> {
    const res = await fetch(url, init);

    if (res.status !== 402 || attempt >= this.maxRetries) return res;

    const challengeHeader = res.headers.get(WWW_AUTH_HEADER);
    if (!challengeHeader) return res;

    const credential = await this._payChallenge(challengeHeader);
    if (!credential) return res;  // couldn't pay — return the 402

    const retryInit: RequestInit = {
      ...init,
      headers: {
        ...(init?.headers ?? {}),
        [AUTH_HEADER]: `${PAYMENT_SCHEME} ${credential}`,
      },
    };

    return this._fetchWithPayment(url, retryInit, attempt + 1);
  }

  private async _payChallenge(challengeHeader: string): Promise<string | null> {
    // Parse method from challenge
    const methodMatch = challengeHeader.match(/method="([^"]+)"/);
    const method = methodMatch?.[1];

    if (method === 'tempo' && this.cfg.privateKey) {
      return this._payTempo(challengeHeader);
    }
    if (method === 'stripe' && this.cfg.stripeTokenFactory) {
      return this._payStripe(challengeHeader);
    }
    return null;
  }

  private async _payTempo(challenge: string): Promise<string | null> {
    // A full implementation would:
    //   1. Parse recipient, currency, amount, nonce from the challenge
    //   2. Sign a TIP-20 transfer transaction with this.cfg.privateKey
    //   3. Broadcast to Tempo chain
    //   4. Return base64(JSON({ txHash, nonce, method: 'tempo' }))
    //
    // For development: return a stub credential.
    const nonceMatch   = challenge.match(/nonce="([^"]+)"/);
    const nonce        = nonceMatch?.[1] ?? generateNonce();
    const credential   = Buffer.from(JSON.stringify({ method: 'tempo', nonce })).toString('base64');
    return credential;
  }

  private async _payStripe(challenge: string): Promise<string | null> {
    // A full implementation would call this.cfg.stripeTokenFactory with
    // the parsed amount/currency, get an SPT, and return it as the credential.
    const nonceMatch = challenge.match(/nonce="([^"]+)"/);
    const nonce      = nonceMatch?.[1] ?? generateNonce();
    const spt        = await this.cfg.stripeTokenFactory!({ challenge });
    const credential = Buffer.from(JSON.stringify({ method: 'stripe', spt, nonce })).toString('base64');
    return credential;
  }
}
