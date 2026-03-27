/**
 * DHT key derivation for the Borgkit capability registry.
 *
 * All keys are SHA-256 hashes of namespaced strings, encoded as CIDv1 (raw
 * codec, 0x55) so they work with libp2p's provider-record API.
 *
 * Key schema:
 *   capability key  →  SHA256("borgkit:cap:<capability>")  → CIDv1
 *   anr value key   →  "/borgkit/anr/" + hex(SHA256("borgkit:anr:<agentId>"))
 *   pid→agentId key →  "/borgkit/pid/" + peerId.toString()
 */

import { sha256 }    from 'multiformats/hashes/sha2';
import { CID }       from 'multiformats/cid';
import { fromString, toString } from 'uint8arrays';

const RAW_CODEC = 0x55;

/**
 * Returns the CID used as the DHT provider-record key for a capability.
 * Cross-language: identical SHA-256 input ensures TypeScript and Rust peers
 * find each other's provider records.
 */
export async function capabilityCid(capability: string): Promise<CID> {
  const input = new TextEncoder().encode(`borgkit:cap:${capability}`);
  const hash  = await sha256.digest(input);
  return CID.createV1(RAW_CODEC, hash);
}

/**
 * Returns the DHT value-record key under which a DiscoveryEntry envelope is
 * stored.  Returned as a Uint8Array suitable for dht.put() / dht.get().
 */
export async function anrDhtKey(agentId: string): Promise<Uint8Array> {
  const input  = new TextEncoder().encode(`borgkit:anr:${agentId}`);
  const hash   = await sha256.digest(input);
  const hex    = toString(hash.bytes, 'hex');
  return fromString(`/borgkit/anr/${hex}`);
}

/**
 * Returns the DHT value-record key that maps a PeerId back to an agentId.
 * Stored as a plain UTF-8 agentId string (authenticity comes from the main
 * ANR record signature, not from this mapping record itself).
 */
export function pidDhtKey(peerIdStr: string): Uint8Array {
  return fromString(`/borgkit/pid/${peerIdStr}`);
}
