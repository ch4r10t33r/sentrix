/**
 * Borgkit x402 Payment Add-on
 * ─────────────────────────────────────────────────────────────────────────────
 * Implements the x402 micropayment protocol (https://x402.org) for
 * agent-to-agent capability pricing.
 *
 * @example Server side — charge callers for capabilities:
 * ```ts
 * import { withX402Payment, usdcBase } from '../addons/x402';
 *
 * const agent = withX402Payment(new MyAgent(), {
 *   pricing: {
 *     premium_search: usdcBase(50, '0xMyWallet', 'Premium search — $0.50'),
 *   },
 * });
 * ```
 *
 * @example Client side — pay automatically:
 * ```ts
 * import { X402Client, MockWalletProvider } from '../addons/x402';
 *
 * const client = new X402Client({ wallet: new MockWalletProvider(), autoPay: true });
 * const resp   = await client.call(agent, req);
 * ```
 */

export * from './types';
export * from './server';
export * from './client';
export * from './facilitator';
