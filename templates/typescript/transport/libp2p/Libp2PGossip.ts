/**
 * Libp2PGossip — IGossipProtocol implementation using libp2p GossipSub.
 *
 * Publishes and subscribes to the /borgkit/gossip/1.0.0 topic.
 * Messages are LP-framed JSON GossipMessage objects.
 */
import type { Libp2p }        from 'libp2p';
import type { GossipMessage } from '../../interfaces/IAgentMesh';

export const GOSSIP_TOPIC = '/borgkit/gossip/1.0.0';

type GossipHandler = (msg: GossipMessage) => Promise<void>;

export class Libp2PGossip {
  private readonly node:     Libp2p;
  private readonly handlers: GossipHandler[] = [];

  constructor(node: Libp2p) {
    this.node = node;
  }

  /** Subscribe to the gossip topic and forward messages to registered handlers. */
  subscribe(): void {
    const pubsub = (this.node.services as Record<string, unknown>)['pubsub'] as any;
    if (!pubsub) throw new Error('GossipSub service not enabled on this libp2p node');

    pubsub.subscribe(GOSSIP_TOPIC);
    pubsub.addEventListener('message', (evt: CustomEvent) => {
      if (evt.detail.topic !== GOSSIP_TOPIC) return;
      try {
        const msg = JSON.parse(new TextDecoder().decode(evt.detail.data)) as GossipMessage;
        // Only forward if TTL allows
        if ((msg.ttl ?? 0) > 0) {
          for (const h of this.handlers) h(msg).catch(() => {});
        }
      } catch { /* malformed message — ignore */ }
    });
  }

  /** Publish a GossipMessage to all subscribed peers. */
  async publish(msg: GossipMessage): Promise<void> {
    const pubsub = (this.node.services as Record<string, unknown>)['pubsub'] as any;
    if (!pubsub) return;
    const data = new TextEncoder().encode(JSON.stringify(msg));
    await pubsub.publish(GOSSIP_TOPIC, data);
  }

  /** Register a handler for incoming gossip messages. */
  onMessage(handler: GossipHandler): void {
    this.handlers.push(handler);
  }

  peers(): string[] {
    const pubsub = (this.node.services as Record<string, unknown>)['pubsub'] as any;
    return pubsub?.getSubscribers(GOSSIP_TOPIC)?.map((p: any) => p.toString()) ?? [];
  }
}
