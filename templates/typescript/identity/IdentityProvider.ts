/**
 * Borgkit Identity Providers
 * ─────────────────────────────────────────────────────────────────────────────
 * Provides flexible agent identity without requiring ERC-8004 on-chain
 * registration or a wallet. Identities produce did:key:z... or
 * did:pkh:eip155:... format W3C DIDs.
 *
 * Identity modes
 * --------------
 * Mode            | Requires wallet? | On-chain? | Use case
 * ----------------|-----------------|-----------|---------------------
 * anonymous       | no              | no        | dev / ephemeral agents
 * localKeystore   | no              | no        | persistent local key (auto-created)
 * env             | no              | no        | containers / cloud (12-factor)
 * rawKey          | no              | no        | bring-your-own hex key
 * erc8004         | yes             | optional  | production, verifiable ownership
 */

import crypto  from 'crypto';
import fs      from 'fs';
import os      from 'os';
import path    from 'path';

// ── helpers ───────────────────────────────────────────────────────────────────

/** Derive Ethereum address from 32-byte private key (secp256k1). */
function ethAddressFromPrivKey(privateKeyHex: string): string {
  try {
    // Use @noble/secp256k1 if available (lightest dep)
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const secp = require('@noble/secp256k1') as typeof import('@noble/secp256k1');
    const { keccak_256 } = require('@noble/hashes/sha3') as typeof import('@noble/hashes/sha3');
    const raw    = privateKeyHex.replace(/^0x/, '');
    const pubKey = secp.getPublicKey(raw, false); // uncompressed 65 bytes
    const pub64  = pubKey.slice(1);               // drop 0x04 prefix → 64 bytes
    const hash   = keccak_256(pub64);
    const addr   = Buffer.from(hash.slice(-20)).toString('hex');
    return '0x' + toEIP55Checksum(addr);
  } catch {
    // Fallback: use Node crypto (no address derivation without secp256k1)
    const raw = privateKeyHex.replace(/^0x/, '');
    return '0x' + crypto.createHash('sha256').update(Buffer.from(raw, 'hex')).digest('hex').slice(-40);
  }
}

function toEIP55Checksum(address: string): string {
  try {
    const { keccak_256 } = require('@noble/hashes/sha3') as typeof import('@noble/hashes/sha3');
    const addr  = address.toLowerCase();
    const hash  = Buffer.from(keccak_256(Buffer.from(addr))).toString('hex');
    return addr.split('').map((c, i) => (parseInt(hash[i], 16) >= 8 ? c.toUpperCase() : c)).join('');
  } catch {
    return address;
  }
}

// ── DID helpers ───────────────────────────────────────────────────────────────

const B58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

function b58encode(data: Uint8Array): string {
  let n = BigInt('0x' + Buffer.from(data).toString('hex'));
  const result: string[] = [];
  while (n > 0n) {
    const rem = Number(n % 58n);
    n = n / 58n;
    result.push(B58_ALPHABET[rem]);
  }
  for (const byte of data) {
    if (byte === 0) result.push('1'); else break;
  }
  return result.reverse().join('');
}

// secp256k1-pub multicodec varint prefix: 0xe7 0x01
const SECP256K1_MULTICODEC = new Uint8Array([0xe7, 0x01]);

function didKeyFromPrivKey(privateKeyHex: string): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const secp = require('@noble/secp256k1') as typeof import('@noble/secp256k1');
    const raw          = privateKeyHex.replace(/^0x/, '');
    const pubCompressed = secp.getPublicKey(raw, true);                 // 33 bytes
    const prefixed      = new Uint8Array(2 + pubCompressed.length);
    prefixed.set(SECP256K1_MULTICODEC);
    prefixed.set(pubCompressed, 2);
    return `did:key:z${b58encode(prefixed)}`;
  } catch {
    // Fallback when @noble/secp256k1 is not installed
    const raw = privateKeyHex.replace(/^0x/, '');
    return `did:key:z${b58encode(new Uint8Array(Buffer.from(raw, 'hex')))}`;
  }
}

function didPkhEvm(address: string, chainId: number): string {
  return `did:pkh:eip155:${chainId}:${address}`;
}

// ── base interface ────────────────────────────────────────────────────────────

export interface IdentityProvider {
  /** Return the Borgkit agent URI (e.g. borgkit://agent/0xABC…) */
  agentId(): string;
  /** Return the owner identifier (Ethereum address or arbitrary string) */
  owner(): string;
  /** Return the 32-byte hex private key (without 0x), or null if not available */
  privateKeyHex(): string | null;
  /** Sign arbitrary bytes; returns hex signature or null if no key */
  signBytes?(data: Buffer): string | null;
  /** Return fields suitable for PluginConfig */
  toPluginConfigFields(): { agentId: string; owner: string; signingKey: string | null };
}

// ── anonymous identity ────────────────────────────────────────────────────────

export class AnonymousIdentity implements IdentityProvider {
  constructor(private readonly name: string = 'unnamed') {}

  agentId() { return `borgkit://agent/${this.name}`; }
  owner()   { return 'anonymous'; }
  privateKeyHex() { return null; }

  toPluginConfigFields() {
    return { agentId: this.agentId(), owner: this.owner(), signingKey: null };
  }
}

// ── raw key identity ──────────────────────────────────────────────────────────

export class RawKeyIdentity implements IdentityProvider {
  private readonly _key: string;
  private readonly _address: string;

  constructor(privateKeyHex: string, private readonly nameOverride?: string) {
    this._key     = privateKeyHex.replace(/^0x/, '');
    if (this._key.length !== 64) throw new Error('privateKeyHex must be 32 bytes (64 hex chars)');
    this._address = ethAddressFromPrivKey(this._key);
  }

  agentId() { return didKeyFromPrivKey(this._key); }

  owner()        { return this._address; }
  privateKeyHex(){ return this._key; }

  toPluginConfigFields() {
    return { agentId: this.agentId(), owner: this.owner(), signingKey: this._key };
  }
}

// ── env key identity ──────────────────────────────────────────────────────────

/**
 * Identity from an environment variable (12-factor / container-friendly).
 *
 * Reads the private key from BORGKIT_AGENT_KEY (default) or a custom env var.
 * Falls back to AnonymousIdentity if the env var is not set.
 *
 * @example
 *   process.env.BORGKIT_AGENT_KEY = '0xdeadbeef...';
 *   const identity = new EnvKeyIdentity();
 */
export class EnvKeyIdentity implements IdentityProvider {
  private readonly _delegate: IdentityProvider;

  constructor(envVar = 'BORGKIT_AGENT_KEY', nameOverride?: string) {
    const val = process.env[envVar];
    if (val) {
      this._delegate = new RawKeyIdentity(val, nameOverride);
    } else {
      console.warn(
        `[Borgkit] ${envVar} not set — using anonymous identity. ` +
        'Set the env var or use LocalKeystoreIdentity for persistent identity.'
      );
      this._delegate = new AnonymousIdentity(nameOverride ?? 'unnamed');
    }
  }

  agentId()       { return this._delegate.agentId(); }
  owner()         { return this._delegate.owner(); }
  privateKeyHex() { return this._delegate.privateKeyHex(); }
  toPluginConfigFields() { return this._delegate.toPluginConfigFields(); }
}

// ── local keystore identity ───────────────────────────────────────────────────

/**
 * Persistent identity stored as a plain-text hex key in ~/.borgkit/keystore/.
 *
 * The key file is created on first use (chmod 0600). The same key is reused on
 * every subsequent run, giving the agent a stable identity across restarts
 * without requiring a wallet or on-chain registration.
 *
 * @example
 *   const identity = new LocalKeystoreIdentity('research-agent');
 *   // Key auto-created at ~/.borgkit/keystore/research-agent.key
 *   console.log(identity.agentId()); // borgkit://agent/0x...
 */
export class LocalKeystoreIdentity implements IdentityProvider {
  private readonly _key: string;
  private readonly _address: string;

  constructor(
    private readonly name: string,
    keystoreDir?: string,
  ) {
    const dir  = keystoreDir ?? path.join(os.homedir(), '.borgkit', 'keystore');
    this._key  = this._loadOrCreate(dir, name);
    this._address = ethAddressFromPrivKey(this._key);
  }

  agentId() { return didKeyFromPrivKey(this._key); }
  owner()   { return this._address; }
  privateKeyHex() { return this._key; }

  toPluginConfigFields() {
    return { agentId: this.agentId(), owner: this.owner(), signingKey: this._key };
  }

  private _loadOrCreate(dir: string, name: string): string {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
    const keyfile = path.join(dir, `${name}.key`);
    if (fs.existsSync(keyfile)) {
      return fs.readFileSync(keyfile, 'utf8').trim();
    }
    // Generate new random key
    const key = crypto.randomBytes(32).toString('hex');
    fs.writeFileSync(keyfile, key, { encoding: 'utf8', mode: 0o600 });
    console.log(`[Borgkit] New identity created: ${keyfile}`);
    return key;
  }
}

// ── ERC-8004 on-chain identity ────────────────────────────────────────────────

export interface ERC8004IdentityConfig {
  privateKeyHex: string;
  chainId?: number;
  contractAddress?: string;
  rpcUrl?: string;
}

/**
 * On-chain identity compliant with ERC-8004.
 *
 * Anchors the agent's ANR record on a smart contract, providing verifiable
 * on-chain ownership.  Requires a wallet private key and gas to register.
 * All other Borgkit features work identically whether you use this mode or
 * an off-chain mode.
 *
 * Note: On-chain registration is OPTIONAL.  You can use this class purely
 * for its key derivation without calling registerOnChain().
 *
 * @example
 *   const identity = new ERC8004Identity({
 *     privateKeyHex: process.env.WALLET_PRIVATE_KEY!,
 *     chainId: 8453,                        // Base
 *     contractAddress: '0x...',
 *     rpcUrl: 'https://mainnet.base.org',
 *   });
 *   await identity.registerOnChain(anrText);
 */
export class ERC8004Identity implements IdentityProvider {
  private readonly _key: string;
  private readonly _address: string;
  private readonly _chainId: number;
  private readonly _contractAddress?: string;
  private readonly _rpcUrl?: string;

  constructor(config: ERC8004IdentityConfig) {
    this._key     = config.privateKeyHex.replace(/^0x/, '');
    if (this._key.length !== 64) throw new Error('privateKeyHex must be 32 bytes');
    this._address         = ethAddressFromPrivKey(this._key);
    this._chainId         = config.chainId ?? 8453;
    this._contractAddress = config.contractAddress;
    this._rpcUrl          = config.rpcUrl;
  }

  agentId() { return didPkhEvm(this._address, this._chainId); }
  owner()   { return this._address; }
  privateKeyHex() { return this._key; }

  toPluginConfigFields() {
    return { agentId: this.agentId(), owner: this.owner(), signingKey: this._key };
  }

  /**
   * Publish the signed ANR to the ERC-8004 on-chain registry.
   * Returns the transaction hash.
   *
   * Requirements:
   *   npm install ethers
   *   A funded wallet with gas on chainId.
   *   A deployed ERC-8004 registry contract.
   */
  async registerOnChain(anrText: string): Promise<string> {
    if (!this._contractAddress || !this._rpcUrl) {
      throw new Error(
        'ERC8004Identity.registerOnChain() requires contractAddress and rpcUrl. ' +
        'See docs/identity.md for setup instructions.'
      );
    }
    try {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const { ethers } = require('ethers') as typeof import('ethers');
      const provider = new ethers.JsonRpcProvider(this._rpcUrl);
      const wallet   = new ethers.Wallet('0x' + this._key, provider);
      const abi      = ['function setRecord(bytes calldata anr) external'];
      const contract = new ethers.Contract(this._contractAddress, abi, wallet);
      const tx       = await contract.setRecord(Buffer.from(anrText));
      await tx.wait();
      return tx.hash;
    } catch (err: any) {
      if (err.code === 'MODULE_NOT_FOUND') {
        throw new Error('On-chain registration requires ethers.js: npm install ethers');
      }
      throw err;
    }
  }
}

// ── factory ───────────────────────────────────────────────────────────────────

export interface IdentityConfig {
  mode: 'anonymous' | 'local' | 'env' | 'raw' | 'erc8004';
  name?: string;
  privateKeyHex?: string;
  envVar?: string;
  keystoreDir?: string;
  chainId?: number;
  contractAddress?: string;
  rpcUrl?: string;
}

/**
 * Factory: pick an identity mode by name.
 *
 * @example
 *   const identity = identityFromConfig({ mode: 'local', name: 'my-agent' });
 */
export function identityFromConfig(config: IdentityConfig): IdentityProvider {
  const { mode, name = 'unnamed', privateKeyHex, envVar, keystoreDir } = config;

  switch (mode) {
    case 'anonymous': return new AnonymousIdentity(name);
    case 'env':       return new EnvKeyIdentity(envVar ?? 'BORGKIT_AGENT_KEY', name);
    case 'raw':
      if (!privateKeyHex) throw new Error("mode='raw' requires privateKeyHex");
      return new RawKeyIdentity(privateKeyHex, name);
    case 'erc8004':
      if (!privateKeyHex) throw new Error("mode='erc8004' requires privateKeyHex");
      return new ERC8004Identity({
        privateKeyHex,
        chainId:         config.chainId,
        contractAddress: config.contractAddress,
        rpcUrl:          config.rpcUrl,
      });
    case 'local':
    default:
      return new LocalKeystoreIdentity(name, keystoreDir);
  }
}
