/**
 * Derives a libp2p PeerId (secp256k1) from an ANR private key.
 *
 * The same 32-byte secp256k1 private key used to sign ANR records is reused
 * as the libp2p identity key.  This means the ANR public key IS the libp2p
 * peer identity — one keypair, one identity, no key management overhead.
 */

import { keys }     from '@libp2p/crypto';
import { peerIdFromPrivateKey } from '@libp2p/peer-id';
import type { PeerId } from '@libp2p/interface';

/**
 * @param rawPrivateKey  32-byte secp256k1 private key (same as ANR signing key)
 * @returns              libp2p PeerId derived from that key
 */
export async function peerIdFromAnrKey(rawPrivateKey: Uint8Array): Promise<PeerId> {
  const privateKey = await keys.generateKeyPairFromSeed('secp256k1', rawPrivateKey);
  return peerIdFromPrivateKey(privateKey);
}

/**
 * @returns  The compressed 33-byte secp256k1 public key for a given private key.
 *           Matches the ANR `a.pk` field exactly.
 */
export async function publicKeyFromAnrKey(rawPrivateKey: Uint8Array): Promise<Uint8Array> {
  const privateKey = await keys.generateKeyPairFromSeed('secp256k1', rawPrivateKey);
  return privateKey.publicKey.raw;
}
