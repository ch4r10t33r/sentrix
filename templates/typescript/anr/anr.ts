/**
 * ANR — Agent Network Record
 * ───────────────────────────────────────────────────────────────────────────
 * Inspired by EIP-778 (Ethereum Node Records).
 *
 * Wire format  : RLP( [sig, seq, k₁, v₁, k₂, v₂, …] )
 * Signed over  : keccak256( RLP( ["anr-v1", seq, k₁, v₁, …] ) )
 * Text form    : "anr:" + base64url(wire bytes, no padding)
 * Max size     : 512 bytes
 * Key ordering : lexicographic, unique, no duplicates
 *
 * Standard ANR keys
 * ─────────────────
 *  Network (inherited from ENR):
 *    id         identity scheme name ("amp-v1")
 *    secp256k1  compressed public key (33 bytes)
 *    ip         IPv4 address (4 bytes)
 *    ip6        IPv6 address (16 bytes)
 *    tcp        TCP port (uint16 big-endian)
 *    udp        UDP port (uint16 big-endian)
 *
 *  Agent-specific  (prefix "a."):
 *    a.id       agent identifier string  e.g. "borgkit://agent/0xABC"
 *    a.name     human-readable name
 *    a.ver      semver string  e.g. "1.2.3"
 *    a.caps     RLP list of capability name strings
 *    a.tags     RLP list of tag strings
 *    a.proto    transport hint  "http" | "ws" | "grpc" | "tcp"
 *    a.port     agent API port (uint16 big-endian)
 *    a.tls      TLS flag  0x00 | 0x01
 *    a.meta     IPFS / Arweave metadata URI
 *    a.owner    owner wallet address (20 bytes)
 *    a.chain    EVM chain ID (uint64 big-endian)
 */

import { keccak256 } from 'ethereum-cryptography/keccak';
import { secp256k1 } from 'ethereum-cryptography/secp256k1';
import { RLP } from '@ethereumjs/rlp';

// ── constants ────────────────────────────────────────────────────────────────

export const ANR_PREFIX   = 'anr:';
export const ANR_ID_SCHEME = 'amp-v1';
export const ANR_MAX_BYTES = 512;
const SIGN_DOMAIN = new TextEncoder().encode('anr-v1');

// ── types ────────────────────────────────────────────────────────────────────

/** All known ANR keys — callers may also use arbitrary string keys. */
export type AnrKey =
  | 'id' | 'secp256k1' | 'ip' | 'ip6' | 'tcp' | 'udp'
  | 'a.id' | 'a.name' | 'a.ver' | 'a.caps' | 'a.tags'
  | 'a.proto' | 'a.port' | 'a.tls' | 'a.meta' | 'a.owner' | 'a.chain'
  | string;

export type AnrKV = Map<string, Uint8Array>;

export interface ANR {
  /** Sequence number — increment on every update */
  seq: bigint;
  /** Key-value pairs (sorted lexicographically before encoding) */
  kv: AnrKV;
  /** 64-byte secp256k1 signature (r‖s) */
  signature: Uint8Array;
}

// ── helpers ──────────────────────────────────────────────────────────────────

function encodeUint16(n: number): Uint8Array {
  const buf = new Uint8Array(2);
  new DataView(buf.buffer).setUint16(0, n, false); // big-endian
  return buf;
}

function encodeUint64(n: bigint): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, n, false);
  return buf;
}

function decodeUint16(b: Uint8Array): number {
  return new DataView(b.buffer, b.byteOffset).getUint16(0, false);
}

function decodeUint64(b: Uint8Array): bigint {
  return new DataView(b.buffer, b.byteOffset).getBigUint64(0, false);
}

function sortedKV(kv: AnrKV): Array<[string, Uint8Array]> {
  return [...kv.entries()].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
}

// ── content (what gets signed) ────────────────────────────────────────────────

function contentRlp(seq: bigint, kv: AnrKV): Uint8Array {
  const pairs = sortedKV(kv).flatMap(([k, v]) => [
    new TextEncoder().encode(k),
    v,
  ]);
  return RLP.encode([SIGN_DOMAIN, encodeUint64(seq), ...pairs]);
}

// ── sign / verify ─────────────────────────────────────────────────────────────

/**
 * Create a signed ANR from a private key.
 * @param privateKey  32-byte secp256k1 private key
 * @param seq         Sequence number
 * @param kv          Key-value map (automatically receives `id` and `secp256k1`)
 */
export function createANR(
  privateKey: Uint8Array,
  seq: bigint,
  kv: Map<string, Uint8Array>
): ANR {
  const pubkey = secp256k1.getPublicKey(privateKey, true); // 33-byte compressed

  kv.set('id',        new TextEncoder().encode(ANR_ID_SCHEME));
  kv.set('secp256k1', pubkey);

  const hash = keccak256(contentRlp(seq, kv));
  const sig  = secp256k1.sign(hash, privateKey);
  const signature = sig.toCompactRawBytes(); // 64 bytes (r‖s)

  return { seq, kv, signature };
}

/**
 * Verify the signature of a decoded ANR.
 * Returns true if valid.
 */
export function verifyANR(record: ANR): boolean {
  const pubkeyBytes = record.kv.get('secp256k1');
  if (!pubkeyBytes) return false;
  try {
    const hash = keccak256(contentRlp(record.seq, record.kv));
    const sig  = secp256k1.Signature.fromCompact(record.signature);
    return secp256k1.verify(sig, hash, pubkeyBytes);
  } catch {
    return false;
  }
}

// ── encode ────────────────────────────────────────────────────────────────────

/**
 * Encode an ANR to its binary RLP wire format.
 */
export function encodeANR(record: ANR): Uint8Array {
  const pairs = sortedKV(record.kv).flatMap(([k, v]) => [
    new TextEncoder().encode(k),
    v,
  ]);
  const wire = RLP.encode([record.signature, encodeUint64(record.seq), ...pairs]);

  if (wire.length > ANR_MAX_BYTES) {
    throw new Error(`ANR exceeds max size: ${wire.length} > ${ANR_MAX_BYTES} bytes`);
  }
  return wire;
}

/**
 * Encode an ANR to its canonical text form: "anr:<base64url>"
 */
export function encodeANRText(record: ANR): string {
  const wire    = encodeANR(record);
  const b64     = Buffer.from(wire).toString('base64url');
  return ANR_PREFIX + b64;
}

// ── decode ────────────────────────────────────────────────────────────────────

/**
 * Decode ANR from raw binary RLP bytes.
 * Throws if structure is invalid.
 */
export function decodeANR(wire: Uint8Array): ANR {
  if (wire.length > ANR_MAX_BYTES) {
    throw new Error(`ANR exceeds max size: ${wire.length} bytes`);
  }

  const list = RLP.decode(wire) as Uint8Array[];
  if (!Array.isArray(list) || list.length < 2 || list.length % 2 !== 0) {
    throw new Error('Invalid ANR RLP structure');
  }

  const [sigBytes, seqBytes, ...rest] = list;
  if (rest.length % 2 !== 0) throw new Error('ANR key-value pairs must be even');

  const kv: AnrKV = new Map();
  for (let i = 0; i < rest.length; i += 2) {
    const key = new TextDecoder().decode(rest[i]);
    kv.set(key, rest[i + 1]);
  }

  return {
    seq:       decodeUint64(seqBytes),
    kv,
    signature: sigBytes,
  };
}

/**
 * Decode ANR from its "anr:<base64url>" text form.
 */
export function decodeANRText(text: string): ANR {
  if (!text.startsWith(ANR_PREFIX)) {
    throw new Error(`ANR text must start with "${ANR_PREFIX}"`);
  }
  const wire = Buffer.from(text.slice(ANR_PREFIX.length), 'base64url');
  return decodeANR(wire);
}

// ── high-level builder ────────────────────────────────────────────────────────

/** Typed builder — avoids raw byte manipulation for common fields. */
export class AnrBuilder {
  private kv: AnrKV = new Map();
  private seq: bigint = 0n;

  setSeq(seq: bigint)             { this.seq = seq; return this; }

  // Agent fields
  setAgentId(id: string)          { this.kv.set('a.id',    enc(id));     return this; }
  setName(name: string)           { this.kv.set('a.name',  enc(name));   return this; }
  setVersion(ver: string)         { this.kv.set('a.ver',   enc(ver));    return this; }
  setCapabilities(caps: string[]) { this.kv.set('a.caps',  RLP.encode(caps.map(enc))); return this; }
  setTags(tags: string[])         { this.kv.set('a.tags',  RLP.encode(tags.map(enc))); return this; }
  setProto(proto: string)         { this.kv.set('a.proto', enc(proto));  return this; }
  setAgentPort(port: number)      { this.kv.set('a.port',  encodeUint16(port)); return this; }
  setTls(tls: boolean)            { this.kv.set('a.tls',   new Uint8Array([tls ? 1 : 0])); return this; }
  setMetaUri(uri: string)         { this.kv.set('a.meta',  enc(uri));    return this; }
  setOwner(addr: Uint8Array)      { this.kv.set('a.owner', addr);        return this; }
  setChainId(id: bigint)          { this.kv.set('a.chain', encodeUint64(id)); return this; }

  // Network fields
  setIpv4(bytes: Uint8Array)      { this.kv.set('ip',  bytes); return this; }
  setIpv6(bytes: Uint8Array)      { this.kv.set('ip6', bytes); return this; }
  setTcpPort(port: number)        { this.kv.set('tcp', encodeUint16(port)); return this; }
  setUdpPort(port: number)        { this.kv.set('udp', encodeUint16(port)); return this; }

  sign(privateKey: Uint8Array): ANR {
    return createANR(privateKey, this.seq, this.kv);
  }
}

// ── decoder helpers ───────────────────────────────────────────────────────────

/** Decode the well-known fields from a verified ANR into a plain object. */
export function parseANRFields(record: ANR): ParsedANR {
  const { kv } = record;
  const get = (k: string) => kv.get(k);
  const str = (k: string) => { const v = get(k); return v ? new TextDecoder().decode(v) : undefined; };

  return {
    seq:          record.seq,
    idScheme:     str('id'),
    pubkey:       get('secp256k1'),
    agentId:      str('a.id'),
    name:         str('a.name'),
    version:      str('a.ver'),
    capabilities: decodeCaps(get('a.caps')),
    tags:         decodeCaps(get('a.tags')),
    proto:        str('a.proto'),
    agentPort:    get('a.port')  ? decodeUint16(get('a.port')!)  : undefined,
    tls:          get('a.tls')   ? get('a.tls')![0] === 1         : false,
    metaUri:      str('a.meta'),
    owner:        get('a.owner'),
    chainId:      get('a.chain') ? decodeUint64(get('a.chain')!) : undefined,
    ip:           get('ip'),
    ip6:          get('ip6'),
    tcpPort:      get('tcp')     ? decodeUint16(get('tcp')!)     : undefined,
    udpPort:      get('udp')     ? decodeUint16(get('udp')!)     : undefined,
  };
}

export interface ParsedANR {
  seq:          bigint;
  idScheme?:    string;
  pubkey?:      Uint8Array;
  agentId?:     string;
  name?:        string;
  version?:     string;
  capabilities: string[];
  tags:         string[];
  proto?:       string;
  agentPort?:   number;
  tls:          boolean;
  metaUri?:     string;
  owner?:       Uint8Array;
  chainId?:     bigint;
  ip?:          Uint8Array;
  ip6?:         Uint8Array;
  tcpPort?:     number;
  udpPort?:     number;
}

// ── internal ──────────────────────────────────────────────────────────────────

function enc(s: string): Uint8Array { return new TextEncoder().encode(s); }

function decodeCaps(raw: Uint8Array | undefined): string[] {
  if (!raw) return [];
  try {
    const list = RLP.decode(raw) as Uint8Array[];
    return list.map(b => new TextDecoder().decode(b));
  } catch {
    return [];
  }
}
