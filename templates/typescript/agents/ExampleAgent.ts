import { IAgent, AgentMetadata }  from '../interfaces/IAgent';
import { AgentRequest }            from '../interfaces/IAgentRequest';
import { AgentResponse }           from '../interfaces/IAgentResponse';
import { DiscoveryEntry,
         IAgentDiscovery }         from '../interfaces/IAgentDiscovery';

/**
 * ExampleAgent — starter template.
 * Replace the capability implementations with your own logic.
 *
 * ── DIDComm v2 encrypted messaging ───────────────────────────────────────────
 * To send or receive end-to-end encrypted messages between agents, use the
 * DidcommClient from `../didcomm`. Example:
 *
 *   import { DidcommClient, MessageTypes } from '../didcomm';
 *
 *   // One-time setup: generate a persistent did:key keypair for this agent
 *   const myKeys = DidcommClient.generateKeyPair();
 *   const client = new DidcommClient(myKeys);
 *
 *   // sendEncrypted: invoke a remote agent over an encrypted DIDComm channel
 *   const recipientDid = 'did:key:z6Mk...'; // obtain from remote agent's ANR
 *   const encrypted = await client.invoke(recipientDid, 'translate', { text: 'hello' });
 *   // → ship `encrypted` (JSON) over HTTP/libp2p to the recipient
 *
 *   // receiveEncrypted: decrypt an incoming envelope (e.g. from POST /didcomm)
 *   const { message, senderDid } = await client.unpack(encrypted);
 *   console.log(message.body);    // { capability: 'translate', input: { text: 'hello' } }
 *   console.log(senderDid);       // 'did:key:z6Mk...' (null for anoncrypt)
 *
 *   // Anonymous send (recipient cannot identify sender):
 *   const anonMsg = await client.invoke(recipientDid, 'ping', {}, { anon: true });
 *
 *   // Reply to an incoming INVOKE:
 *   const reply = await client.respond(senderDid!, message.id, { status: 'ok' });
 */
export class ExampleAgent implements IAgent {
  // ─── ERC-8004 Identity ────────────────────────────────────────────────────
  readonly agentId     = 'borgkit://agent/example';
  readonly owner       = '0xYourWalletAddress';
  readonly metadataUri = 'ipfs://QmYourMetadataHashHere';
  readonly metadata: AgentMetadata = {
    name:        'ExampleAgent',
    version:     '0.1.0',
    description: 'A starter Borgkit agent',
    tags:        ['example', 'starter'],
  };

  // ─── Internal state ───────────────────────────────────────────────────────
  private _registry: IAgentDiscovery | null = null;
  private _p2pInfo: { peerId: string; multiaddr: string } | null = null;

  // ─── Capabilities ─────────────────────────────────────────────────────────
  getCapabilities(): string[] {
    return ['echo', 'ping'];
  }

  // ─── Request handling ─────────────────────────────────────────────────────
  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    // Optional: enforce permissions
    if (this.checkPermission) {
      const allowed = await this.checkPermission(req.from, req.capability);
      if (!allowed) {
        return { requestId: req.requestId, status: 'error', errorMessage: 'Permission denied' };
      }
    }

    switch (req.capability) {
      case 'echo':
        return {
          requestId: req.requestId,
          status:    'success',
          result:    { echo: req.payload },
          timestamp: Date.now(),
        };

      case 'ping':
        return {
          requestId: req.requestId,
          status:    'success',
          result:    { pong: true, agentId: this.agentId },
          timestamp: Date.now(),
        };

      default:
        return {
          requestId:    req.requestId,
          status:       'error',
          errorMessage: `Unknown capability: "${req.capability}"`,
        };
    }
  }

  // ─── Discovery ────────────────────────────────────────────────────────────
  async registerDiscovery(): Promise<void> {
    const { DiscoveryFactory } = await import('../discovery/DiscoveryFactory');

    // Honour BORGKIT_DISCOVERY_TYPE for libp2p / onchain backends; the factory
    // already handles BORGKIT_DISCOVERY_URL → http and defaults to local.
    const discoveryType = process.env['BORGKIT_DISCOVERY_TYPE'];
    this._registry = await DiscoveryFactory.create(
      discoveryType === 'libp2p'  ? { type: 'libp2p' }  :
      discoveryType === 'onchain' ? { type: 'onchain' } :
      {}
    );

    // Capture libp2p node info so getAnr() can populate peerId + multiaddr
    if ('getNodeInfo' in this._registry && typeof (this._registry as any).getNodeInfo === 'function') {
      const info = (this._registry as any).getNodeInfo() as { peerId: string; multiaddr: string } | null;
      if (info?.peerId) this._p2pInfo = info;
    }

    await this._registry.register(this.getAnr());
    console.log(`[ExampleAgent] registered with discovery layer`);
  }

  async unregisterDiscovery(): Promise<void> {
    await this._registry?.unregister(this.agentId);
  }

  // ─── ANR / Identity exposure ──────────────────────────────────────────────
  getAnr(): DiscoveryEntry {
    const host     = process.env['BORGKIT_HOST'] ?? 'localhost';
    const port     = parseInt(process.env['BORGKIT_PORT'] ?? '6174', 10);
    const tls      = (process.env['BORGKIT_TLS'] ?? 'false').toLowerCase() === 'true';

    // Build multiaddr when peerId is known (libp2p mode)
    const peerId    = this._p2pInfo?.peerId ?? null;
    const multiaddr = this._p2pInfo?.multiaddr ??
      (peerId ? `/${tls ? 'dns4' : 'ip4'}/${host}/tcp/${port}/p2p/${peerId}` : undefined);

    const protocol = peerId ? 'libp2p' : (tls ? 'https' : 'http') as any;

    return {
      agentId:      this.agentId,
      name:         this.metadata.name,
      owner:        this.owner,
      capabilities: this.getCapabilities(),
      network:      { protocol, host, port, tls, ...(peerId && { peerId }), ...(multiaddr && { multiaddr }) },
      health:       { status: 'healthy', lastHeartbeat: new Date().toISOString(), uptimeSeconds: 0 },
      registeredAt: new Date().toISOString(),
      metadataUri:  this.metadataUri,
    };
  }

  async getPeerId(): Promise<string | null> {
    return this._p2pInfo?.peerId ?? null;
  }

  // ─── Permissions ──────────────────────────────────────────────────────────
  async checkPermission(caller: string, capability: string): Promise<boolean> {
    // 1. Owner is always allowed
    if (caller === this.owner || caller === this.agentId) return true;

    // 2. Capability-specific allow-list (opt-in via BORGKIT_PERMITTED_CALLERS env var)
    //    Format: "cap1:caller1,caller2;cap2:caller3"
    const permittedRaw = process.env['BORGKIT_PERMITTED_CALLERS'];
    if (permittedRaw) {
      const capMap = parsePermittedCallers(permittedRaw);
      const allowed = capMap.get(capability) ?? capMap.get('*') ?? null;
      if (allowed !== null) return allowed.has(caller) || allowed.has('*');
    }

    // 3. Optional ERC-8004 on-chain delegation check
    //    Activated by BORGKIT_REGISTRY_ADDRESS env var
    const registryAddress = process.env['BORGKIT_REGISTRY_ADDRESS'];
    if (registryAddress) {
      return this._checkOnChainDelegation(caller, capability, registryAddress);
    }

    // 4. Default: open (dev mode)
    return true;
  }

  // ─── Signing ──────────────────────────────────────────────────────────────
  async signMessage(message: string): Promise<string> {
    const privKeyHex = process.env['BORGKIT_AGENT_KEY'];
    if (!privKeyHex) {
      throw new Error('signMessage: no signing key — set BORGKIT_AGENT_KEY=<hex-private-key>');
    }
    const { ethers } = await import('ethers');
    const wallet = new ethers.Wallet(
      privKeyHex.startsWith('0x') ? privKeyHex : '0x' + privKeyHex
    );
    return wallet.signMessage(message);
  }

  // ─── Private helpers ──────────────────────────────────────────────────────

  private async _checkOnChainDelegation(
    caller: string,
    capability: string,
    registryAddress: string,
  ): Promise<boolean> {
    // Dynamic import so ethers is not a hard dependency
    const ethersModule = await import('ethers').catch(() => null);
    if (!ethersModule) return true; // ethers not installed — permissive fallback

    const { ethers } = ethersModule;
    const rpcUrl = process.env['BORGKIT_RPC_URL'];
    if (!rpcUrl) return true; // no RPC configured — permissive fallback

    try {
      const provider = new ethers.JsonRpcProvider(rpcUrl);
      const abi = [
        'function isDelegated(address owner, address delegate, string capability) external view returns (bool)',
      ];
      const contract = new ethers.Contract(registryAddress, abi, provider);
      return await contract['isDelegated'](this.owner, caller, capability) as boolean;
    } catch (err) {
      console.warn('[ExampleAgent] on-chain delegation check failed:', err);
      return true; // permissive fallback on error
    }
  }
}

// ─── Module-level helpers ─────────────────────────────────────────────────────

/**
 * Parse a BORGKIT_PERMITTED_CALLERS string into a capability → caller-set map.
 *
 * Format:  "cap1:caller1,caller2;cap2:caller3"
 * Special: capability segment "*" matches all capabilities.
 */
function parsePermittedCallers(raw: string): Map<string, Set<string>> {
  const result = new Map<string, Set<string>>();
  for (const segment of raw.split(';')) {
    const colonIdx = segment.indexOf(':');
    if (colonIdx === -1) continue;
    const cap     = segment.slice(0, colonIdx).trim();
    const callers = segment.slice(colonIdx + 1).split(',').map(s => s.trim()).filter(Boolean);
    if (cap && callers.length > 0) {
      result.set(cap, new Set(callers));
    }
  }
  return result;
}

// ── Entry point ───────────────────────────────────────────────────────────────
//
// Run directly:
//   npx ts-node agents/ExampleAgent.ts
//   BORGKIT_PORT=9090 npx ts-node agents/ExampleAgent.ts
//
// Or via borgkit-cli:
//   borgkit run ExampleAgent --port 6174
//
if (require.main === module) {
  (async () => {
    const { serve } = await import('../server');
    const port = parseInt(process.env.BORGKIT_PORT ?? '6174', 10);
    const agent = new ExampleAgent();
    await serve(agent, { port });
  })();
}
