/**
 * DHT value-record envelope for DiscoveryEntry.
 *
 * Wire format (JSON, UTF-8 bytes):
 * {
 *   "v":     1,
 *   "seq":   42,
 *   "entry": { ...DiscoveryEntry... },
 *   "sig":   "<base64url 64-byte compact secp256k1 signature>"
 * }
 *
 * Signed bytes: keccak256( "borgkit:anr:v1:" + JSON.stringify(entry) )
 *
 * Recipients MUST:
 *   1. Verify the signature against the owner's public key (from entry.owner or ANR).
 *   2. Reject envelopes where seq ≤ any previously seen seq for that agentId.
 *   3. Apply staleness: if lastHeartbeat > 15 min ago → mark unhealthy.
 */

import { secp256k1 }      from '@noble/curves/secp256k1';
import { keccak_256 }     from '@noble/hashes/sha3';
import { bytesToHex, hexToBytes } from '@noble/hashes/utils';
import { toString, fromString }   from 'uint8arrays';
import { DiscoveryEntry }         from '../../interfaces/IAgentDiscovery';

const SIGN_PREFIX = 'borgkit:anr:v1:';
const ENVELOPE_VERSION = 1;

export interface DhtEnvelope {
  v:     number;
  seq:   number;
  entry: DiscoveryEntry;
  sig:   string;   // base64url compact secp256k1 signature (64 bytes)
}

/**
 * Serialise a DiscoveryEntry + seq into the signed DHT envelope bytes.
 * @param entry       The DiscoveryEntry to publish.
 * @param seq         Monotonically increasing sequence number.
 * @param privateKey  Raw 32-byte secp256k1 private key (same as ANR key).
 */
export function encodeEnvelope(
  entry:      DiscoveryEntry,
  seq:        number,
  privateKey: Uint8Array,
): Uint8Array {
  const entryJson  = JSON.stringify(entry);
  const msgBytes   = new TextEncoder().encode(`${SIGN_PREFIX}${entryJson}`);
  const msgHash    = keccak_256(msgBytes);
  const sig        = secp256k1.sign(msgHash, privateKey);
  const sigB64     = toString(sig.toCompactRawBytes(), 'base64url');

  const envelope: DhtEnvelope = { v: ENVELOPE_VERSION, seq, entry, sig: sigB64 };
  return new TextEncoder().encode(JSON.stringify(envelope));
}

/**
 * Deserialise and optionally verify a DHT envelope.
 * Returns null if parsing fails; does NOT throw.
 * Signature verification is optional (pass `publicKey` to enable).
 */
export function decodeEnvelope(
  raw:       Uint8Array,
  publicKey?: Uint8Array,   // compressed 33-byte secp256k1 public key
): { entry: DiscoveryEntry; seq: number } | null {
  try {
    const env: DhtEnvelope = JSON.parse(new TextDecoder().decode(raw));
    if (env.v !== ENVELOPE_VERSION || !env.entry || typeof env.seq !== 'number') {
      return null;
    }
    if (publicKey) {
      const entryJson = JSON.stringify(env.entry);
      const msgBytes  = new TextEncoder().encode(`${SIGN_PREFIX}${entryJson}`);
      const msgHash   = keccak_256(msgBytes);
      const sigBytes  = fromString(env.sig, 'base64url');
      const sig       = secp256k1.Signature.fromCompact(sigBytes);
      if (!secp256k1.verify(sig, msgHash, publicKey)) return null;
    }
    return { entry: applyStalenessMark(env.entry), seq: env.seq };
  } catch {
    return null;
  }
}

/**
 * Apply staleness heuristic at read time:
 *   > 15 min since lastHeartbeat → unhealthy
 *   > 5 min  since lastHeartbeat → degraded
 */
function applyStalenessMark(entry: DiscoveryEntry): DiscoveryEntry {
  const last  = new Date(entry.health.lastHeartbeat).getTime();
  const delta = Date.now() - last;
  if (delta > 15 * 60 * 1000) {
    return { ...entry, health: { ...entry.health, status: 'unhealthy' } };
  }
  if (delta > 5 * 60 * 1000) {
    return { ...entry, health: { ...entry.health, status: 'degraded' } };
  }
  return entry;
}
