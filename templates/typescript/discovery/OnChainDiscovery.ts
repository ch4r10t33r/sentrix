/**
 * OnChainDiscovery — ERC-8004 on-chain agent registry adapter.
 *
 * Backed by an ERC-8004 compliant smart contract deployed on any EVM chain.
 * Uses ethers.js v6 for provider / signer / contract interaction.
 *
 * Read-only mode (no privateKey):  query / listAll / find / findById work.
 * Read-write mode (privateKey set): register / unregister / heartbeat also work.
 *
 * Prerequisites:
 *   npm install ethers
 */

import { ethers } from 'ethers';
import { IAgentDiscovery, DiscoveryEntry } from '../interfaces/IAgentDiscovery';

// ── ABI ────────────────────────────────────────────────────────────────────────

const ERC8004_ABI = [
  'function registerAgent(string agentId, string name, string owner, string[] capabilities, string protocol, string host, uint256 port, bool tls, string metadataUri) external',
  'function unregisterAgent(string agentId) external',
  'function heartbeat(string agentId) external',
  'function getAgent(string agentId) external view returns (tuple(string agentId, string name, string owner, string[] capabilities, string protocol, string host, uint256 port, bool tls, uint256 registeredAt, uint256 lastHeartbeat, string metadataUri, bool active))',
  'function queryByCapability(string capability) external view returns (tuple(string agentId, string name, string owner, string[] capabilities, string protocol, string host, uint256 port, bool tls, uint256 registeredAt, uint256 lastHeartbeat, string metadataUri, bool active)[])',
  'function listAll() external view returns (tuple(string agentId, string name, string owner, string[] capabilities, string protocol, string host, uint256 port, bool tls, uint256 registeredAt, uint256 lastHeartbeat, string metadataUri, bool active)[])',
  'event AgentRegistered(string indexed agentId, string name, address indexed owner, string[] capabilities)',
  'event AgentUnregistered(string indexed agentId)',
  'event AgentHeartbeat(string indexed agentId, uint256 timestamp)',
];

// ── Config ─────────────────────────────────────────────────────────────────────

export interface OnChainDiscoveryConfig {
  /** JSON-RPC endpoint. Default: BORGKIT_RPC_URL env var */
  rpcUrl?: string;
  /** Deployed ERC-8004 registry contract address. Default: BORGKIT_CONTRACT_ADDRESS env var */
  contractAddress?: string;
  /** Hex private key for signing transactions. Omit for read-only mode. Default: BORGKIT_PRIVATE_KEY env var */
  privateKey?: string;
  /** Chain ID. Default: 8453 (Base mainnet) */
  chainId?: number;
  /** Gas limit for write operations. Default: 300_000 */
  gasLimit?: number;
  /** Heartbeat interval in ms. Default: 30_000 */
  heartbeatIntervalMs?: number;
}

// Internal fully-resolved config (all fields present)
type ResolvedConfig = Required<OnChainDiscoveryConfig>;

// ── Health thresholds ──────────────────────────────────────────────────────────

const HEALTHY_MS   = 15 * 60 * 1000; // 15 minutes
const DEGRADED_MS  = 30 * 60 * 1000; // 30 minutes

// ── OnChainDiscovery ───────────────────────────────────────────────────────────

export class OnChainDiscovery implements IAgentDiscovery {
  private readonly provider:   ethers.JsonRpcProvider;
  private readonly contract:   ethers.Contract;        // read-only view
  private readonly signer:     ethers.Wallet | null;   // null = read-only
  private readonly rwContract: ethers.Contract | null; // contract connected to signer
  private readonly config:     ResolvedConfig;

  private _heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private _registeredId:   string | null = null;

  constructor(cfg: OnChainDiscoveryConfig = {}) {
    const rpcUrl = cfg.rpcUrl
      ?? process.env['BORGKIT_RPC_URL']
      ?? '';
    const contractAddress = cfg.contractAddress
      ?? process.env['BORGKIT_CONTRACT_ADDRESS']
      ?? '';
    const privateKey = cfg.privateKey
      ?? process.env['BORGKIT_PRIVATE_KEY']
      ?? '';

    if (!rpcUrl) {
      throw new Error(
        '[OnChainDiscovery] rpcUrl is required. Pass it in config or set BORGKIT_RPC_URL.'
      );
    }
    if (!contractAddress) {
      throw new Error(
        '[OnChainDiscovery] contractAddress is required. Pass it in config or set BORGKIT_CONTRACT_ADDRESS.'
      );
    }

    this.config = {
      rpcUrl,
      contractAddress,
      privateKey,
      chainId:              cfg.chainId              ?? 8453,
      gasLimit:             cfg.gasLimit             ?? 300_000,
      heartbeatIntervalMs:  cfg.heartbeatIntervalMs  ?? 30_000,
    };

    this.provider = new ethers.JsonRpcProvider(
      this.config.rpcUrl,
      this.config.chainId
    );

    // Read-only contract (provider only)
    this.contract = new ethers.Contract(
      this.config.contractAddress,
      ERC8004_ABI,
      this.provider
    );

    // Signer + read-write contract (only when a private key is provided)
    if (this.config.privateKey) {
      this.signer     = new ethers.Wallet(this.config.privateKey, this.provider);
      this.rwContract = this.contract.connect(this.signer) as ethers.Contract;
    } else {
      this.signer     = null;
      this.rwContract = null;
    }
  }

  // ── Write operations ─────────────────────────────────────────────────────────

  async register(entry: DiscoveryEntry): Promise<void> {
    this.assertReadWrite('register');
    try {
      const tx = await this.rwContract!['registerAgent'](
        entry.agentId,
        entry.name,
        entry.owner,
        entry.capabilities,
        entry.network.protocol,
        entry.network.host,
        entry.network.port,
        entry.network.tls,
        entry.metadataUri ?? '',
        { gasLimit: this.config.gasLimit }
      );
      await tx.wait();
    } catch (err) {
      throw new Error(`OnChainDiscovery.register failed: ${(err as Error).message}`);
    }

    // Track the registered agent id and start heartbeat
    this._registeredId = entry.agentId;
    if (this.config.heartbeatIntervalMs > 0) {
      this._heartbeatTimer = setInterval(
        () => this.heartbeat(entry.agentId).catch(console.warn),
        this.config.heartbeatIntervalMs
      );
    }
    console.log(`[OnChainDiscovery] Registered: ${entry.agentId} → contract ${this.config.contractAddress}`);
  }

  async unregister(agentId: string): Promise<void> {
    this.assertReadWrite('unregister');

    // Clear heartbeat timer before the on-chain call
    if (this._heartbeatTimer !== null) {
      clearInterval(this._heartbeatTimer);
      this._heartbeatTimer = null;
    }
    if (this._registeredId === agentId) {
      this._registeredId = null;
    }

    try {
      const tx = await this.rwContract!['unregisterAgent'](
        agentId,
        { gasLimit: this.config.gasLimit }
      );
      await tx.wait();
    } catch (err) {
      throw new Error(`OnChainDiscovery.unregister failed: ${(err as Error).message}`);
    }
    console.log(`[OnChainDiscovery] Unregistered: ${agentId}`);
  }

  async heartbeat(agentId: string): Promise<void> {
    this.assertReadWrite('heartbeat');
    try {
      const tx = await this.rwContract!['heartbeat'](
        agentId,
        { gasLimit: this.config.gasLimit }
      );
      await tx.wait();
    } catch (err) {
      throw new Error(`OnChainDiscovery.heartbeat failed: ${(err as Error).message}`);
    }
  }

  // ── Read operations ──────────────────────────────────────────────────────────

  async query(capability: string): Promise<DiscoveryEntry[]> {
    try {
      const records: any[] = await this.contract['queryByCapability'](capability);
      return records.map(r => this.recordToEntry(r));
    } catch (err) {
      throw new Error(`OnChainDiscovery.query failed: ${(err as Error).message}`);
    }
  }

  async listAll(): Promise<DiscoveryEntry[]> {
    try {
      const records: any[] = await this.contract['listAll']();
      return records.map(r => this.recordToEntry(r));
    } catch (err) {
      throw new Error(`OnChainDiscovery.listAll failed: ${(err as Error).message}`);
    }
  }

  // ── Convenience methods ──────────────────────────────────────────────────────

  /**
   * Returns the first healthy agent for `capability`, falling back to the
   * first entry (regardless of health) if none are healthy.
   */
  async find(capability: string): Promise<DiscoveryEntry | null> {
    const entries = await this.query(capability);
    if (entries.length === 0) return null;
    const healthy = entries.find(e => e.health.status === 'healthy');
    return healthy ?? entries[0];
  }

  /**
   * Look up an agent by exact agentId.
   */
  async findById(agentId: string): Promise<DiscoveryEntry | null> {
    try {
      const record: any = await this.contract['getAgent'](agentId);
      // getAgent returns an empty-ish struct when the agent is unknown;
      // treat an empty agentId as "not found".
      if (!record || !record.agentId) return null;
      return this.recordToEntry(record);
    } catch (err) {
      throw new Error(`OnChainDiscovery.findById failed: ${(err as Error).message}`);
    }
  }

  // ── Private helpers ──────────────────────────────────────────────────────────

  /**
   * Map a raw contract AgentRecord tuple to a DiscoveryEntry.
   */
  private recordToEntry(r: any): DiscoveryEntry {
    const registeredAtMs  = Number(r.registeredAt)   * 1000;
    const lastHeartbeatMs = Number(r.lastHeartbeat)  * 1000;
    const now             = Date.now();
    const age             = now - lastHeartbeatMs;

    let status: DiscoveryEntry['health']['status'];
    if (!r.active) {
      status = 'unhealthy';
    } else if (age <= HEALTHY_MS) {
      status = 'healthy';
    } else if (age <= DEGRADED_MS) {
      status = 'degraded';
    } else {
      status = 'unhealthy';
    }

    const uptimeSeconds = registeredAtMs > 0
      ? Math.max(0, Math.floor((now - registeredAtMs) / 1000))
      : 0;

    return {
      agentId:      r.agentId,
      name:         r.name,
      owner:        r.owner,
      capabilities: [...r.capabilities],
      network: {
        protocol: r.protocol as DiscoveryEntry['network']['protocol'],
        host:     r.host,
        port:     Number(r.port),
        tls:      r.tls,
      },
      metadataUri:  r.metadataUri || undefined,
      health: {
        status,
        lastHeartbeat: new Date(lastHeartbeatMs).toISOString(),
        uptimeSeconds,
      },
      registeredAt: new Date(registeredAtMs).toISOString(),
    };
  }

  /**
   * Guard: throw a clear error when a write operation is attempted in
   * read-only mode (no private key configured).
   */
  private assertReadWrite(operation: string): void {
    if (!this.rwContract) {
      throw new Error(
        `OnChainDiscovery.${operation} requires a privateKey. ` +
        'Pass it in config or set BORGKIT_PRIVATE_KEY.'
      );
    }
  }
}
