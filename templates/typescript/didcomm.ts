/**
 * DIDComm v2 encrypted messaging for Borgkit agents.
 *
 * Implements authenticated (authcrypt) and anonymous (anoncrypt) encryption
 * between DID-identified agents using X25519 ECDH + XSalsa20-Poly1305.
 *
 * Crypto stack:
 *   - Key agreement : X25519 (via tweetnacl box keypair)
 *   - AEAD          : XSalsa20-Poly1305 (nacl.box / nacl.secretbox)
 *   - Encoding      : base64url (custom), base58btc (custom, Bitcoin alphabet)
 *   - DID method    : did:key (multicodec x25519-pub prefix 0xec 0x01)
 *
 * Dependencies: tweetnacl, tweetnacl-util
 * Install: npm install tweetnacl tweetnacl-util
 *
 * Usage example (Alice encrypts an INVOKE to Bob, Bob decrypts):
 *
 *   import { DidcommClient, MessageTypes } from './didcomm';
 *
 *   const alice = DidcommClient.generateKeyPair();
 *   const bob   = DidcommClient.generateKeyPair();
 *
 *   const aliceClient = new DidcommClient(alice);
 *   const bobClient   = new DidcommClient(bob);
 *
 *   // Alice invokes Bob's "translate" capability
 *   const encrypted = await aliceClient.invoke(bob.did, 'translate', { text: 'hello' });
 *
 *   // Bob decrypts
 *   const { message, senderDid } = await bobClient.unpack(encrypted);
 *   console.log(message.body);   // { capability: 'translate', input: { text: 'hello' } }
 *   console.log(senderDid);      // alice.did
 */

// ── Runtime detection ────────────────────────────────────────────────────────
// tweetnacl is a CommonJS module; use a dynamic import shim for ESM builds.
// In Node / bundlers this resolves to the same module.
import * as nacl from 'tweetnacl';

// ── Types ─────────────────────────────────────────────────────────────────────

/**
 * An X25519 keypair with its derived did:key DID.
 */
export interface DidKeyPair {
  /** Full DID string: "did:key:z6Mk..." (x25519-pub multicodec) */
  did: string;
  /** X25519 public key (32 bytes) */
  publicKey: Uint8Array;
  /** X25519 private key (32 bytes) */
  privateKey: Uint8Array;
}

/**
 * A plaintext DIDComm v2 message (JWM wire-format body).
 */
export interface DidcommMessage {
  /** UUID v4 message identifier */
  id: string;
  /** Message type URI, e.g. MessageTypes.INVOKE */
  type: string;
  /** Sender DID — omit for anoncrypt */
  from?: string;
  /** Recipient DIDs */
  to: string[];
  /** Unix timestamp (seconds) when the message was created */
  created_time: number;
  /** Unix timestamp (seconds) after which the message should be rejected */
  expires_time?: number;
  /** Application-level body payload */
  body: Record<string, unknown>;
  /** Optional binary/JSON attachments */
  attachments?: Attachment[];
}

/**
 * A DIDComm v2 attachment descriptor.
 */
export interface Attachment {
  id: string;
  data: { base64?: string; json?: unknown };
  media_type?: string;
}

/**
 * JWE JSON serialization of an encrypted DIDComm message.
 */
export interface EncryptedMessage {
  /** Base64url-encoded ciphertext */
  ciphertext: string;
  /** Base64url-encoded protected header JSON */
  protected: string;
  /** Per-recipient encrypted content keys */
  recipients: Recipient[];
  /** Base64url-encoded nonce / IV (24 bytes for XSalsa20) */
  iv: string;
  /** Base64url-encoded authentication tag */
  tag: string;
  /** Unprotected header (optional, plaintext metadata) */
  unprotected?: Record<string, unknown>;
}

/**
 * Per-recipient header in the JWE envelope.
 */
export interface Recipient {
  /** kid is the recipient's DID key fragment: "did:key:z...#key-1" */
  header: { kid: string };
  /** Base64url-encoded wrapped (encrypted) copy of the content key */
  encrypted_key: string;
}

// ── Message type constants ─────────────────────────────────────────────────────

/** Well-known DIDComm v2 message type URIs for Borgkit. */
export const MessageTypes = {
  INVOKE:   'https://borgkit.dev/didcomm/1.0/invoke',
  RESPONSE: 'https://borgkit.dev/didcomm/1.0/response',
  FORWARD:  'https://borgkit.dev/didcomm/1.0/forward',
  PING:     'https://borgkit.dev/didcomm/1.0/ping',
  PONG:     'https://borgkit.dev/didcomm/1.0/pong',
} as const;

// ── Crypto / encoding helpers ─────────────────────────────────────────────────

/** Bitcoin / base58btc alphabet (used by did:key multibase 'z' prefix). */
const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

/** Encode bytes to base58btc (no multibase prefix). */
function base58Encode(bytes: Uint8Array): string {
  let num = BigInt(0);
  for (const byte of bytes) {
    num = num * BigInt(256) + BigInt(byte);
  }
  let result = '';
  while (num > BigInt(0)) {
    const rem = Number(num % BigInt(58));
    num = num / BigInt(58);
    result = BASE58_ALPHABET[rem] + result;
  }
  // Leading zero bytes → leading '1's
  for (const byte of bytes) {
    if (byte !== 0) break;
    result = '1' + result;
  }
  return result;
}

/** Decode base58btc string (no multibase prefix) to bytes. */
function base58Decode(str: string): Uint8Array {
  let num = BigInt(0);
  for (const ch of str) {
    const idx = BASE58_ALPHABET.indexOf(ch);
    if (idx === -1) throw new Error(`Invalid base58 character: ${ch}`);
    num = num * BigInt(58) + BigInt(idx);
  }
  const bytes: number[] = [];
  while (num > BigInt(0)) {
    bytes.unshift(Number(num % BigInt(256)));
    num = num / BigInt(256);
  }
  // Restore leading zero bytes
  for (const ch of str) {
    if (ch !== '1') break;
    bytes.unshift(0);
  }
  return new Uint8Array(bytes);
}

/** Base64url encode (no padding). */
function b64uEncode(bytes: Uint8Array): string {
  // Use Buffer in Node, btoa in browsers
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary)
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=/g, '');
}

/** Base64url decode (with or without padding). */
function b64uDecode(str: string): Uint8Array {
  const padded = str.replace(/-/g, '+').replace(/_/g, '/');
  const pad = (4 - (padded.length % 4)) % 4;
  const b64 = padded + '='.repeat(pad);
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/**
 * Multicodec prefix for X25519 public key: 0xec 0x01.
 * See https://github.com/multiformats/multicodec
 */
const MULTICODEC_X25519_PUB = new Uint8Array([0xec, 0x01]);

/**
 * Encode a 32-byte X25519 public key as a did:key DID.
 *
 * Format: "did:key:z" + base58btc(0xec 0x01 || pubkey)
 */
function encodeDidKey(pubKey: Uint8Array): string {
  const prefixed = new Uint8Array(MULTICODEC_X25519_PUB.length + pubKey.length);
  prefixed.set(MULTICODEC_X25519_PUB);
  prefixed.set(pubKey, MULTICODEC_X25519_PUB.length);
  return `did:key:z${base58Encode(prefixed)}`;
}

/**
 * Decode the raw 32-byte X25519 public key from a did:key DID.
 *
 * Supports both x25519-pub (0xec 0x01) and ed25519-pub (0xed 0x01) prefixes,
 * as some implementations use Ed25519 keys for did:key.
 */
function decodeDidKey(did: string): Uint8Array {
  if (!did.startsWith('did:key:z')) {
    throw new Error(`Not a did:key DID (multibase 'z'): ${did}`);
  }
  const encoded = did.slice('did:key:z'.length);
  const decoded = base58Decode(encoded);
  // Strip the 2-byte multicodec prefix
  if (decoded.length < 34) {
    throw new Error(`did:key payload too short: ${decoded.length} bytes`);
  }
  return decoded.slice(2); // 32-byte key
}

/** Generate a UUID v4 (crypto-random). */
function uuidV4(): string {
  const bytes = nacl.randomBytes(16);
  bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
  bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant bits
  const hex = Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
  return [
    hex.slice(0, 8),
    hex.slice(8, 12),
    hex.slice(12, 16),
    hex.slice(16, 20),
    hex.slice(20),
  ].join('-');
}

/** Current Unix time in seconds. */
function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}

// ── Protected header helpers ──────────────────────────────────────────────────

interface ProtectedHeader {
  alg: string;          // 'ECDH-1PU+XSalsa20Poly1305' | 'ECDH+XSalsa20Poly1305'
  enc: string;          // 'XSalsa20Poly1305'
  skid?: string;        // sender DID (authcrypt only)
  epk?: string;         // base64url(ephemeral_pub) for anoncrypt
}

// ── DidcommClient ─────────────────────────────────────────────────────────────

/**
 * DIDComm v2 client for sending and receiving encrypted messages.
 *
 * Each client is bound to a single keypair / DID identity.
 */
export class DidcommClient {
  constructor(private readonly keyPair: DidKeyPair) {}

  // ── Static factory helpers ───────────────────────────────────────────────

  /**
   * Generate a fresh X25519 keypair and derive the corresponding did:key DID.
   *
   * @returns A DidKeyPair ready for use with DidcommClient.
   */
  static generateKeyPair(): DidKeyPair {
    const kp = nacl.box.keyPair();
    return {
      did:        encodeDidKey(kp.publicKey),
      publicKey:  kp.publicKey,
      privateKey: kp.secretKey,
    };
  }

  /**
   * Resolve the raw 32-byte X25519 public key from a did:key DID string.
   *
   * @param did - A did:key DID, e.g. "did:key:z6Mk..."
   */
  static resolvePublicKey(did: string): Uint8Array {
    return decodeDidKey(did);
  }

  // ── Encryption ───────────────────────────────────────────────────────────

  /**
   * Authcrypt: encrypt *message* for *recipientDids* while authenticating the sender.
   *
   * Each recipient receives a separately wrapped copy of the content key,
   * derived via X25519 ECDH between the sender's static key and the recipient's
   * public key. The protected header includes the sender's DID (`skid`).
   *
   * @param message       - Plaintext DIDComm message to encrypt.
   * @param recipientDids - Array of recipient DID strings.
   */
  async packAuthcrypt(
    message: DidcommMessage,
    recipientDids: string[],
  ): Promise<EncryptedMessage> {
    return this._pack(message, recipientDids, false);
  }

  /**
   * Anoncrypt: encrypt *message* for *recipientDids* without revealing the sender.
   *
   * Uses a freshly generated ephemeral keypair as the "sender" for each pack
   * operation. Recipients cannot determine who sent the message.
   *
   * @param message       - Plaintext DIDComm message.
   * @param recipientDids - Array of recipient DID strings.
   */
  async packAnoncrypt(
    message: DidcommMessage,
    recipientDids: string[],
  ): Promise<EncryptedMessage> {
    return this._pack(message, recipientDids, true);
  }

  /**
   * Decrypt an incoming EncryptedMessage addressed to this client's keypair.
   *
   * @param encrypted - JWE envelope to decrypt.
   * @returns The plaintext DidcommMessage and the sender's DID (null for anoncrypt).
   * @throws  If no recipient entry matches this client's key, or if decryption fails.
   */
  async unpack(
    encrypted: EncryptedMessage,
  ): Promise<{ message: DidcommMessage; senderDid: string | null }> {
    // 1. Decode protected header
    const headerJson = new TextDecoder().decode(b64uDecode(encrypted.protected));
    const header: ProtectedHeader = JSON.parse(headerJson);
    const isAnon = header.alg === 'ECDH+XSalsa20Poly1305';

    // 2. Determine the ephemeral / sender public key used for ECDH
    let senderPub: Uint8Array;
    if (isAnon) {
      if (!header.epk) throw new Error('Anoncrypt envelope missing epk in protected header');
      senderPub = b64uDecode(header.epk);
    } else {
      if (!header.skid) throw new Error('Authcrypt envelope missing skid in protected header');
      senderPub = decodeDidKey(header.skid);
    }

    // 3. Find the recipient entry for our key (match by KID)
    const myKid = `${this.keyPair.did}#key-1`;
    const recipientEntry = encrypted.recipients.find(r => r.header.kid === myKid);
    if (!recipientEntry) {
      throw new Error(`No recipient entry found for key ${myKid}`);
    }

    // 4. ECDH: derive shared secret → unwrap content key
    //    nacl.box.open uses the sender's pubkey + our private key to derive
    //    the shared secret and decrypt. The encrypted_key is a nacl.box ciphertext.
    const encryptedKeyBytes = b64uDecode(recipientEntry.encrypted_key);
    const nonce24 = encryptedKeyBytes.slice(0, 24);         // prepended nonce
    const boxCiphertext = encryptedKeyBytes.slice(24);
    const contentKey = nacl.box.open(boxCiphertext, nonce24, senderPub, this.keyPair.privateKey);
    if (!contentKey) throw new Error('Content key decryption failed (bad key or tampered data)');

    // 5. Decrypt body with content key (nacl.secretbox = XSalsa20-Poly1305)
    const iv = b64uDecode(encrypted.iv);
    const tag = b64uDecode(encrypted.tag);
    // Reconstitute nacl secretbox ciphertext: authenticator (tag) || ciphertext
    const bodyCiphertext = b64uDecode(encrypted.ciphertext);
    const secretboxCt = new Uint8Array(tag.length + bodyCiphertext.length);
    secretboxCt.set(tag);
    secretboxCt.set(bodyCiphertext, tag.length);

    const plaintext = nacl.secretbox.open(secretboxCt, iv, contentKey);
    if (!plaintext) throw new Error('Body decryption failed (bad content key or tampered data)');

    const msg: DidcommMessage = JSON.parse(new TextDecoder().decode(plaintext));
    const senderDid: string | null = isAnon ? null : (header.skid ?? null);

    return { message: msg, senderDid };
  }

  // ── Signing (JWS) ────────────────────────────────────────────────────────

  /**
   * Sign a DIDComm message (not encrypted) using Ed25519.
   *
   * Because tweetnacl box keypairs are X25519, this method re-interprets the
   * private key bytes as an Ed25519 seed (same 32-byte scalar). In production,
   * maintain separate X25519 and Ed25519 keypairs.
   *
   * @param message - Plaintext message to sign.
   * @returns Compact JWS string: base64url(header).base64url(payload).base64url(sig)
   */
  sign(message: DidcommMessage): string {
    const signingKp = nacl.sign.keyPair.fromSeed(this.keyPair.privateKey);
    const payload = b64uEncode(new TextEncoder().encode(JSON.stringify(message)));
    const header  = b64uEncode(new TextEncoder().encode(JSON.stringify({
      alg: 'EdDSA',
      kid: `${this.keyPair.did}#key-1`,
    })));
    const sigInput = new TextEncoder().encode(`${header}.${payload}`);
    const sig = nacl.sign.detached(sigInput, signingKp.secretKey);
    return `${header}.${payload}.${b64uEncode(sig)}`;
  }

  /**
   * Verify a compact JWS produced by {@link sign}.
   *
   * @param jws - Compact JWS string.
   * @returns The embedded DIDComm message and the signer's DID.
   * @throws  If the signature is invalid or the JWS is malformed.
   */
  static verify(jws: string): { message: DidcommMessage; signerDid: string } {
    const parts = jws.split('.');
    if (parts.length !== 3) throw new Error('Invalid JWS: expected 3 parts');
    const [headerB64, payloadB64, sigB64] = parts;

    const header  = JSON.parse(new TextDecoder().decode(b64uDecode(headerB64)));
    const signerDid = (header.kid as string).split('#')[0];
    const pubKey  = decodeDidKey(signerDid);

    // Re-derive Ed25519 pubkey from the X25519 pubkey bytes (same 32-byte scalar convention)
    // For a real implementation, store Ed25519 pubkeys separately.
    const sigInput = new TextEncoder().encode(`${headerB64}.${payloadB64}`);
    const sig      = b64uDecode(sigB64);

    const verifyKp = nacl.sign.keyPair.fromSeed(pubKey); // seed == X25519 pub bytes
    const valid = nacl.sign.detached.verify(sigInput, sig, verifyKp.publicKey);
    if (!valid) throw new Error('JWS signature verification failed');

    const message: DidcommMessage = JSON.parse(
      new TextDecoder().decode(b64uDecode(payloadB64)),
    );
    return { message, signerDid };
  }

  // ── Convenience builders ─────────────────────────────────────────────────

  /**
   * Build and encrypt an INVOKE message to *recipientDid*.
   *
   * @param recipientDid     - Target agent's DID.
   * @param capability       - Capability name to invoke.
   * @param input            - Input payload for the capability.
   * @param opts.anon        - Use anoncrypt (default: false → authcrypt).
   * @param opts.expiresInSeconds - TTL in seconds from now.
   */
  async invoke(
    recipientDid: string,
    capability: string,
    input: Record<string, unknown>,
    opts: { anon?: boolean; expiresInSeconds?: number } = {},
  ): Promise<EncryptedMessage> {
    const msg: DidcommMessage = {
      id:           uuidV4(),
      type:         MessageTypes.INVOKE,
      from:         opts.anon ? undefined : this.keyPair.did,
      to:           [recipientDid],
      created_time: nowSecs(),
      expires_time: opts.expiresInSeconds ? nowSecs() + opts.expiresInSeconds : undefined,
      body:         { capability, input },
    };
    return opts.anon
      ? this.packAnoncrypt(msg, [recipientDid])
      : this.packAuthcrypt(msg, [recipientDid]);
  }

  /**
   * Build and encrypt a RESPONSE message to *recipientDid*.
   *
   * @param recipientDid - Recipient's DID.
   * @param replyToId    - The `id` of the INVOKE message this replies to.
   * @param output       - Result payload.
   */
  async respond(
    recipientDid: string,
    replyToId: string,
    output: Record<string, unknown>,
  ): Promise<EncryptedMessage> {
    const msg: DidcommMessage = {
      id:           uuidV4(),
      type:         MessageTypes.RESPONSE,
      from:         this.keyPair.did,
      to:           [recipientDid],
      created_time: nowSecs(),
      body:         { reply_to: replyToId, output },
    };
    return this.packAuthcrypt(msg, [recipientDid]);
  }

  // ── Internal helpers ─────────────────────────────────────────────────────

  /**
   * Shared encryption implementation for both authcrypt and anoncrypt.
   *
   * Protocol steps:
   *   1. Generate a random 32-byte content key and 24-byte nonce.
   *   2. Encrypt the plaintext message with nacl.secretbox (content key).
   *   3. For each recipient: ECDH(sender_priv, recipient_pub) → shared secret,
   *      then nacl.box-encrypt the content key for the recipient.
   *   4. Assemble the JWE envelope.
   */
  private _pack(
    message: DidcommMessage,
    recipientDids: string[],
    anon: boolean,
  ): EncryptedMessage {
    // Step 1 — content key + nonce
    const contentKey = nacl.randomBytes(32);
    const bodyNonce  = nacl.randomBytes(24); // XSalsa20 nonce = 24 bytes

    // Step 2 — encrypt body
    const plaintext  = new TextEncoder().encode(JSON.stringify(message));
    const secretboxCt = nacl.secretbox(plaintext, bodyNonce, contentKey);
    // nacl.secretbox output = Poly1305 tag (16 bytes) || ciphertext
    const tag        = secretboxCt.slice(0, 16);
    const ciphertext = secretboxCt.slice(16);

    // Step 3 — per-recipient key wrapping
    let senderPub: Uint8Array;
    let senderPriv: Uint8Array;
    if (anon) {
      const ephKp  = nacl.box.keyPair();
      senderPub    = ephKp.publicKey;
      senderPriv   = ephKp.secretKey;
    } else {
      senderPub    = this.keyPair.publicKey;
      senderPriv   = this.keyPair.privateKey;
    }

    const recipients: Recipient[] = recipientDids.map(did => {
      const recipientPub   = decodeDidKey(did);
      const keyNonce       = nacl.randomBytes(24);
      const wrappedKey     = nacl.box(contentKey, keyNonce, recipientPub, senderPriv);
      // Prepend nonce so the decryptor can recover it (nonce || boxCiphertext)
      const encryptedKey   = new Uint8Array(keyNonce.length + wrappedKey.length);
      encryptedKey.set(keyNonce);
      encryptedKey.set(wrappedKey, keyNonce.length);
      return {
        header:        { kid: `${did}#key-1` },
        encrypted_key: b64uEncode(encryptedKey),
      };
    });

    // Step 4 — protected header
    const protectedHeader: ProtectedHeader = anon
      ? { alg: 'ECDH+XSalsa20Poly1305',    enc: 'XSalsa20Poly1305', epk: b64uEncode(senderPub) }
      : { alg: 'ECDH-1PU+XSalsa20Poly1305', enc: 'XSalsa20Poly1305', skid: this.keyPair.did };

    return {
      ciphertext: b64uEncode(ciphertext),
      protected:  b64uEncode(new TextEncoder().encode(JSON.stringify(protectedHeader))),
      recipients,
      iv:         b64uEncode(bodyNonce),
      tag:        b64uEncode(tag),
    };
  }
}
