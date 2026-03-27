/**
 * Borgkit Identity Providers
 * ─────────────────────────────────────────────────────────────────────────────
 * Flexible identity options — ERC-8004 on-chain registration is optional.
 *
 * Quick start (no wallet required):
 *   import { LocalKeystoreIdentity } from './identity';
 *   const identity = new LocalKeystoreIdentity('my-agent'); // auto-creates key
 *
 * With environment variable:
 *   import { EnvKeyIdentity } from './identity';
 *   const identity = new EnvKeyIdentity(); // reads BORGKIT_AGENT_KEY
 *
 * On-chain (optional, requires wallet + gas):
 *   import { ERC8004Identity } from './identity';
 *   const identity = new ERC8004Identity({ privateKeyHex: '0x...' });
 */

export * from './IdentityProvider';
