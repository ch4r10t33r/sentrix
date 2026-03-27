/**
 * x402 Server Middleware
 * ─────────────────────────────────────────────────────────────────────────────
 * Wraps any IAgent to enforce x402 payment before handling paid capabilities.
 *
 * @example
 * ```ts
 * import { withX402Payment } from '../addons/x402/server';
 * import { usdcBase }        from '../addons/x402/types';
 * import { MyAgent }         from './MyAgent';
 *
 * const agent = withX402Payment(new MyAgent(), {
 *   pricing: {
 *     generate_image: usdcBase(50, '0xMyWallet', 'Image gen — $0.50'),
 *     translate_text: { network: 'base', asset: '0x...', amount: '100000', payTo: '0xMyWallet' },
 *   },
 * });
 *
 * // agent is now an IAgent that returns payment_required for paid capabilities
 * await agent.registerDiscovery?.();
 * ```
 *
 * Custom verification:
 * ```ts
 * const agent = withX402Payment(new MyAgent(), {
 *   pricing: { ... },
 *   verify: async (payment, requirements) => {
 *     const fac = new X402Facilitator();
 *     return fac.verify(payment, requirements);
 *   },
 * });
 * ```
 */

import type { IAgent }           from '../../interfaces/IAgent';
import type { AgentRequest }     from '../../interfaces/IAgentRequest';
import type { AgentResponse }    from '../../interfaces/IAgentResponse';
import type {
  CapabilityPricing,
  X402Payment,
  X402PaymentRequirements,
  X402Receipt,
}                                from './types';
import { toRequirements }        from './types';

// ── middleware options ────────────────────────────────────────────────────────

export interface X402ServerOptions {
  /** Map of capability name → pricing config */
  pricing: Record<string, CapabilityPricing>;
  /**
   * Custom payment verifier.
   * Defaults to DEV MODE (accepts all proofs with a warning).
   * Override with X402Facilitator.verify or your own on-chain check.
   */
  verify?: (payment: X402Payment, requirements: X402PaymentRequirements) => Promise<X402Receipt>;
  /**
   * Set to true to reject all unverified proofs (no dev-mode fallback).
   * Default: false.
   */
  strict?: boolean;
}

// ── dev-mode verifier ─────────────────────────────────────────────────────────

function devModeVerifier(strict: boolean) {
  return async (_payment: X402Payment, _requirements: X402PaymentRequirements): Promise<X402Receipt> => {
    if (strict) {
      return {
        success:     false,
        errorReason: 'strict=true but no verifier configured. Provide a verify() function.',
      };
    }
    console.warn(
      '[x402] DEV MODE: payment proof accepted without verification. ' +
      'Provide a verify() option or use X402Facilitator before going to production.'
    );
    return { success: true };
  };
}

// ── factory function ──────────────────────────────────────────────────────────

/**
 * Wrap an IAgent with x402 payment enforcement.
 * Returns a new IAgent that intercepts handle_request for priced capabilities.
 */
export function withX402Payment(agent: IAgent, options: X402ServerOptions): IAgent {
  const { pricing, verify, strict = false } = options;
  const verifyFn = verify ?? devModeVerifier(strict);

  return {
    ...agent,

    async handleRequest(req: AgentRequest): Promise<AgentResponse> {
      const pricingConfig = pricing[req.capability];

      if (pricingConfig) {
        const payment = (req as any).x402 as X402Payment | undefined;

        if (!payment) {
          // No payment proof — return payment_required
          const requirements = toRequirements(pricingConfig, req.requestId);
          return {
            requestId:            req.requestId,
            status:               'payment_required',
            errorMessage:         `Capability '${req.capability}' requires payment.`,
            paymentRequirements:  [requirements],
          } as AgentResponse & { paymentRequirements: X402PaymentRequirements[] };
        }

        // Verify the proof
        const requirements = toRequirements(pricingConfig, req.requestId);
        const receipt       = await verifyFn(payment, requirements);

        if (!receipt.success) {
          return {
            requestId:    req.requestId,
            status:       'error',
            errorMessage: `x402 payment verification failed: ${receipt.errorReason ?? 'unknown'}`,
          };
        }
      }

      // Free capability or verified payment — delegate to original agent
      return agent.handleRequest(req);
    },
  };
}

// ── class-based alternative ───────────────────────────────────────────────────

/**
 * Class-based alternative to withX402Payment().
 * Extend this class instead of implementing IAgent directly.
 *
 * @example
 * ```ts
 * class MyPaidAgent extends X402Agent {
 *   readonly agentId = 'borgkit://agent/my-paid-agent';
 *   readonly owner   = '0xMyWallet';
 *
 *   protected pricing = {
 *     generate_image: usdcBase(50, '0xMyWallet'),
 *   };
 *
 *   getCapabilities() { return ['generate_image']; }
 *
 *   protected async handlePaidRequest(req: AgentRequest): Promise<AgentResponse> {
 *     if (req.capability === 'generate_image') {
 *       return { requestId: req.requestId, status: 'success', result: { url: '...' } };
 *     }
 *     return { requestId: req.requestId, status: 'error', errorMessage: 'Unknown capability' };
 *   }
 * }
 * ```
 */
export abstract class X402Agent implements IAgent {
  abstract readonly agentId: string;
  abstract readonly owner: string;

  /** Override with your capability pricing table. */
  protected pricing: Record<string, CapabilityPricing> = {};

  /** Set to true to reject unverified proofs (no dev-mode fallback). */
  protected x402Strict = false;

  abstract getCapabilities(): string[];

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    const pricingConfig = this.pricing[req.capability];

    if (pricingConfig) {
      const payment = (req as any).x402 as X402Payment | undefined;
      if (!payment) {
        const requirements = toRequirements(pricingConfig, req.requestId);
        return {
          requestId:           req.requestId,
          status:              'payment_required' as any,
          errorMessage:        `Capability '${req.capability}' requires payment.`,
          paymentRequirements: [requirements],
        } as any;
      }

      const requirements = toRequirements(pricingConfig, req.requestId);
      const receipt      = await this.verifyX402Payment(payment, requirements);
      if (!receipt.success) {
        return {
          requestId:    req.requestId,
          status:       'error',
          errorMessage: `x402 payment verification failed: ${receipt.errorReason ?? 'unknown'}`,
        };
      }
    }

    return this.handlePaidRequest(req);
  }

  /**
   * Override to add real payment verification.
   * Default: DEV MODE — accepts all proofs with a warning.
   */
  protected async verifyX402Payment(
    _payment: X402Payment,
    _requirements: X402PaymentRequirements,
  ): Promise<X402Receipt> {
    if (this.x402Strict) {
      return {
        success:     false,
        errorReason: 'x402Strict=true but verifyX402Payment() not overridden.',
      };
    }
    console.warn(
      '[x402] DEV MODE: payment accepted without verification. ' +
      'Override verifyX402Payment() before production.'
    );
    return { success: true };
  }

  /** Implement your capability logic here (called after payment is verified). */
  protected abstract handlePaidRequest(req: AgentRequest): Promise<AgentResponse>;
}
