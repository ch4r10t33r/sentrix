/**
 * Libp2PNode — creates and manages a js-libp2p host for Sentrix.
 *
 * Wraps libp2p node lifecycle: start, stop, peer info.
 * Used by both Libp2PListener (server side) and Libp2PAgentClient (client side).
 */
import { createLibp2p, type Libp2p } from 'libp2p';
import { tcp } from '@libp2p/tcp';
import { noise } from '@chainsafe/libp2p-noise';
import { yamux } from '@chainsafe/libp2p-yamux';
import { identify } from '@libp2p/identify';
import { gossipsub } from '@chainsafe/libp2p-gossipsub';

export interface Libp2PNodeOptions {
  /** TCP listen address. Default: /ip4/0.0.0.0/tcp/6174 */
  listenAddrs?: string[];
  /** Enable GossipSub. Default: true */
  gossip?: boolean;
}

export interface Libp2PNodeInfo {
  peerId:    string;
  multiaddr: string;
}

export class Libp2PNode {
  private node: Libp2p | null = null;
  private readonly opts: Libp2PNodeOptions;

  constructor(opts: Libp2PNodeOptions = {}) {
    this.opts = opts;
  }

  async start(): Promise<Libp2PNodeInfo> {
    const services: Record<string, unknown> = {
      identify: identify(),
    };
    if (this.opts.gossip !== false) {
      services['pubsub'] = gossipsub({ allowPublishToZeroTopicPeers: true });
    }

    this.node = await createLibp2p({
      addresses: {
        listen: this.opts.listenAddrs ?? ['/ip4/0.0.0.0/tcp/6174'],
      },
      transports:  [tcp()],
      streamMuxers: [yamux()],
      connectionEncrypters: [noise()],
      services,
    });

    await this.node.start();

    const peerId    = this.node.peerId.toString();
    const addrs     = this.node.getMultiaddrs();
    const multiaddr = addrs.length > 0 ? addrs[0].toString() : '';

    return { peerId, multiaddr };
  }

  async stop(): Promise<void> {
    await this.node?.stop();
    this.node = null;
  }

  get libp2p(): Libp2p {
    if (!this.node) throw new Error('Libp2PNode not started');
    return this.node;
  }

  get peerId(): string {
    return this.node?.peerId.toString() ?? '';
  }

  get multiaddrs(): string[] {
    return this.node?.getMultiaddrs().map(m => m.toString()) ?? [];
  }
}
