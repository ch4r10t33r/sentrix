/**
 * Discovery layer interface.
 * Implement this to plug into any backend:
 *   - LocalDiscovery   → in-memory (dev / testing)
 *   - HttpDiscovery    → REST-based registry
 *   - GossipDiscovery  → P2P gossip protocol
 *   - OnChainDiscovery → ERC-8004 on-chain registry
 */
export interface IAgentDiscovery {
  /**
   * Register an agent with its capability list.
   * `network` carries the agent's reachable address.
   */
  register(entry: DiscoveryEntry): Promise<void>;

  /**
   * Remove an agent from the discovery layer.
   */
  unregister(agentId: string): Promise<void>;

  /**
   * Find all agents that expose a given capability.
   * Returns a list of DiscoveryEntry objects.
   */
  query(capability: string): Promise<DiscoveryEntry[]>;

  /**
   * List every registered agent (no filter).
   */
  listAll(): Promise<DiscoveryEntry[]>;

  /**
   * Emit a heartbeat so the registry knows the agent is alive.
   */
  heartbeat(agentId: string): Promise<void>;

  /**
   * Find the best healthy agent for `capability`.
   * Convenience method — implementations should delegate to `query()`.
   */
  find?(capability: string): Promise<DiscoveryEntry | null>;

  /**
   * Look up an agent by exact agentId.
   * Convenience method — implementations should delegate to `listAll()`.
   */
  findById?(agentId: string): Promise<DiscoveryEntry | null>;
}

export interface DiscoveryEntry {
  agentId: string;
  name: string;
  owner: string;
  capabilities: string[];
  network: {
    protocol: 'http' | 'websocket' | 'grpc' | 'tcp' | 'libp2p';
    host: string;
    port: number;
    tls: boolean;
    peerId?: string;
    multiaddr?: string;
  };
  metadataUri?: string;
  health: {
    status: 'healthy' | 'degraded' | 'unhealthy';
    lastHeartbeat: string; // ISO 8601
    uptimeSeconds: number;
  };
  registeredAt: string; // ISO 8601
}
