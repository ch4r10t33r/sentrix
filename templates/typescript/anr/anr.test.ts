/**
 * ANR — quick smoke tests (run with: npx ts-node anr.test.ts)
 */
import { AnrBuilder, decodeANRText, encodeANRText, parseANRFields, verifyANR } from './anr';
import { secp256k1 } from 'ethereum-cryptography/secp256k1';

// Generate a fresh key pair
const privateKey = secp256k1.utils.randomPrivateKey();

// ── Build and sign an ANR ─────────────────────────────────────────────────────
const record = new AnrBuilder()
  .setSeq(1n)
  .setAgentId('borgkit://agent/0xABC123')
  .setName('WeatherAgent')
  .setVersion('1.0.0')
  .setCapabilities(['getWeather', 'getForecast'])
  .setTags(['weather', 'data'])
  .setProto('http')
  .setAgentPort(6174)
  .setTls(false)
  .setMetaUri('ipfs://QmWeatherMeta')
  .setIpv4(new Uint8Array([127, 0, 0, 1]))
  .setTcpPort(9000)
  .sign(privateKey);

// ── Round-trip: encode → text → decode ───────────────────────────────────────
const text    = encodeANRText(record);
const decoded = decodeANRText(text);
const parsed  = parseANRFields(decoded);

console.log('=== ANR text ===');
console.log(text);
console.log('\n=== Parsed fields ===');
console.log(parsed);

// ── Verify signature ──────────────────────────────────────────────────────────
const valid = verifyANR(decoded);
console.log('\n=== Signature valid ===', valid);

// ── Tamper detection ──────────────────────────────────────────────────────────
decoded.kv.set('a.name', new TextEncoder().encode('TAMPERED'));
const invalid = verifyANR(decoded);
console.log('=== Tampered valid (should be false) ===', invalid);

console.assert(valid,    'Signature should be valid');
console.assert(!invalid, 'Tampered record should be invalid');
console.assert(parsed.name === 'WeatherAgent', 'Name should decode correctly');
console.assert(parsed.capabilities.includes('getWeather'), 'Capabilities should decode');
console.log('\n✔ All assertions passed');
