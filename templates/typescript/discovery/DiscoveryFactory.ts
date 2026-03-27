/**
 * DiscoveryFactory — selects the appropriate discovery backend.
 *
 * Priority order (highest → lowest):
 *   1. Explicit `type` in config
 *   2. BORGKIT_DISCOVERY_URL env var    → HttpDiscovery
 *   3. default                          → LocalDiscovery
 *
 * Usage:
 *   const registry = await DiscoveryFactory.create({ type: 'libp2p', libp2p: { privateKey } });
 *   await registry.register(entry);
 */

import { IAgentDiscovery } from '../interfaces/IAgentDiscovery';
import { LocalDiscovery }  from './LocalDiscovery';
import { HttpDiscovery }   from './HttpDiscovery';
import type { OnChainDiscoveryConfig } from './OnChainDiscovery';

export type DiscoveryType = 'local' | 'http' | 'libp2p' | 'onchain';

export interface DiscoveryConfig {
  type?: DiscoveryType;

  /** Required when type === 'http' */
  http?: {
    baseUrl: string;
    apiKey?: string;
    timeoutMs?: number;
    heartbeatIntervalMs?: number;
  };

  /** Required when type === 'onchain' */
  onchain?: OnChainDiscoveryConfig;

  /**
   * Required when type === 'libp2p'.
   * All fields are optional — sensible defaults apply for development.
   */
  libp2p?: {
    /**
     * 32-byte raw secp256k1 private key (same key used to sign ANR records).
     * Omit only for ephemeral / throwaway nodes.
     */
    privateKey?: Uint8Array;
    /** Multiaddrs to listen on. Default: ['/ip4/0.0.0.0/udp/0/quic-v1'] */
    listenAddresses?: string[];
    /**
     * Known bootstrap peer multiaddrs.
     * Also read from BORGKIT_BOOTSTRAP_PEERS env var (comma-separated).
     */
    bootstrapPeers?: string[];
    /** DHT record re-publish interval in ms. Default: 30_000 */
    heartbeatIntervalMs?: number;
    /** Enable mDNS for local network discovery. Default: true */
    enableMdns?: boolean;
    /**
     * DHT client mode — participates in discovery but does not store records
     * for others.  Default: false
     */
    dhtClientMode?: boolean;
  };
}

export class DiscoveryFactory {
  /**
   * Create a discovery backend.
   * Returns a Promise because the libp2p backend requires async initialisation.
   */
  static async create(config: DiscoveryConfig = {}): Promise<IAgentDiscovery> {
    const type = config.type
      ?? (process.env['BORGKIT_DISCOVERY_URL'] ? 'http' : 'local');

    switch (type) {
      case 'http': {
        const url = config.http?.baseUrl ?? process.env['BORGKIT_DISCOVERY_URL'];
        if (!url) throw new Error('[DiscoveryFactory] http type requires baseUrl or BORGKIT_DISCOVERY_URL');
        return new HttpDiscovery({
          baseUrl:              url,
          apiKey:               config.http?.apiKey              ?? process.env['BORGKIT_DISCOVERY_KEY'],
          timeoutMs:            config.http?.timeoutMs,
          heartbeatIntervalMs:  config.http?.heartbeatIntervalMs,
        });
      }

      case 'libp2p': {
        // Lazy import so consumers that don't use libp2p don't pay the import cost
        const { Libp2pDiscovery } = await import('./Libp2pDiscovery');
        return Libp2pDiscovery.create(config.libp2p ?? {});
      }

      case 'onchain': {
        const { OnChainDiscovery } = await import('./OnChainDiscovery');
        return new OnChainDiscovery(config.onchain ?? {});
      }

      case 'local':
      default:
        return LocalDiscovery.getInstance();
    }
  }
}
