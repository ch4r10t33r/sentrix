/**
 * borgkit inspect — inspect ANR records and mesh topology.
 *
 * Subcommands:
 *   borgkit inspect anr <anr-text>            Decode and display an ANR record
 *   borgkit inspect agents [--host] [--port]  List all agents in the mesh
 *   borgkit inspect agent <agent-id>          Inspect a specific agent
 *   borgkit inspect capabilities              List all capabilities in the mesh
 */

import http from 'http';
import { logger } from '../utils/logger';

// ── ANR constants (mirrors templates/typescript/anr/anr.ts) ──────────────────

const ANR_PREFIX    = 'anr:';
const ANR_KEY_NAMES: Record<string, string> = {
  'id':        'id-scheme',
  'secp256k1': 'public-key (secp256k1)',
  'ip':        'IPv4',
  'ip6':       'IPv6',
  'tcp':       'TCP port',
  'udp':       'UDP port',
  'a.id':      'agent-id',
  'a.name':    'name',
  'a.ver':     'version',
  'a.caps':    'capabilities (RLP)',
  'a.tags':    'tags (RLP)',
  'a.proto':   'protocol',
  'a.port':    'agent port',
  'a.tls':     'TLS',
  'a.meta':    'metadata URI',
  'a.owner':   'owner',
  'a.chain':   'chain ID',
};

// ── subcommand dispatcher ─────────────────────────────────────────────────────

export interface InspectOptions {
  host?:       string;
  port?:       string;
  capability?: string;
  raw?:        boolean;
}

export async function inspectCommand(
  subcommand: string | undefined,
  target:     string | undefined,
  options:    InspectOptions,
): Promise<void> {
  switch (subcommand) {
    case 'anr':
      if (!target) {
        logger.error('Usage: borgkit inspect anr <anr-text>');
        process.exit(1);
      }
      return inspectAnr(target, options);

    case 'agents':
      return inspectAgents(options);

    case 'agent':
      if (!target) {
        logger.error('Usage: borgkit inspect agent <agent-id>');
        process.exit(1);
      }
      return inspectAgent(target, options);

    case 'capabilities':
      return inspectCapabilities(options);

    default:
      logger.title('borgkit inspect — available subcommands');
      console.log('');
      console.log('  borgkit inspect anr <anr-text>           Decode an ANR record');
      console.log('  borgkit inspect agents                   List all mesh agents');
      console.log('  borgkit inspect agent <agent-id>         Inspect one agent');
      console.log('  borgkit inspect capabilities             All mesh capabilities');
      console.log('');
      console.log('Options:');
      console.log('  --host <host>   Discovery/agent host  (default: localhost)');
      console.log('  --port <port>   Discovery/agent port  (default: 3000 for agents, 8080 for agent)');
      console.log('  --raw           Print raw JSON');
      console.log('');
  }
}

// ── ANR decoder ───────────────────────────────────────────────────────────────

function inspectAnr(anrText: string, options: InspectOptions): void {
  if (!anrText.startsWith(ANR_PREFIX)) {
    logger.error(`ANR text must start with "${ANR_PREFIX}"`);
    process.exit(1);
  }

  let raw: Buffer;
  try {
    const b64 = anrText.slice(ANR_PREFIX.length).replace(/-/g, '+').replace(/_/g, '/');
    const padded = b64 + '='.repeat((4 - b64.length % 4) % 4);
    raw = Buffer.from(padded, 'base64');
  } catch {
    logger.error('Failed to decode ANR base64');
    process.exit(1);
  }

  try {
    const decoded = rlpDecode(raw);

    if (options.raw) {
      // Print as hex key-value pairs
      console.log(JSON.stringify(
        { raw: raw.toString('hex'), decoded: formatRlpForJson(decoded) },
        null, 2,
      ));
      return;
    }

    if (!Array.isArray(decoded) || decoded.length < 2) {
      logger.error('ANR decode: expected RLP list with at least [sig, seq, ...kv]');
      process.exit(1);
    }

    const [sigBuf, seqBuf, ...rest] = decoded as Buffer[];

    logger.title('ANR Record');
    console.log('');

    // Signature
    const sigHex = Buffer.isBuffer(sigBuf) ? sigBuf.toString('hex') : '?';
    console.log(`  ${'signature'.padEnd(22)} ${sigHex.slice(0, 32)}...`);

    // Sequence
    const seq = Buffer.isBuffer(seqBuf) ? readUint64BE(seqBuf) : 0;
    console.log(`  ${'sequence'.padEnd(22)} ${seq}`);
    console.log('');

    // Key-value pairs
    for (let i = 0; i + 1 < rest.length; i += 2) {
      const keyBuf = rest[i];
      const valBuf = rest[i + 1];
      if (!Buffer.isBuffer(keyBuf) || !Buffer.isBuffer(valBuf)) continue;

      const key     = keyBuf.toString('utf8');
      const label   = ANR_KEY_NAMES[key] ?? key;
      const display = formatAnrValue(key, valBuf);
      console.log(`  ${label.padEnd(22)} ${display}`);
    }

    console.log('');
    console.log(`  ${'size'.padEnd(22)} ${raw.length} / 512 bytes`);
    if (raw.length > 400) {
      logger.warn(`  Record is ${raw.length} bytes — approaching 512-byte limit`);
    }
  } catch (e) {
    logger.error(`ANR decode failed: ${e}`);
    process.exit(1);
  }
}

function formatAnrValue(key: string, buf: Buffer): string {
  // Port fields (2-byte big-endian uint16)
  if (['tcp', 'udp', 'a.port'].includes(key)) {
    return buf.length >= 2 ? buf.readUInt16BE(0).toString() : buf.toString('hex');
  }
  // Chain ID (8-byte big-endian uint64)
  if (key === 'a.chain') {
    return buf.length >= 8 ? readUint64BE(buf).toString() : buf.toString('hex');
  }
  // Boolean (TLS)
  if (key === 'a.tls') {
    return buf.length > 0 ? (buf[0] === 1 ? 'true' : 'false') : 'false';
  }
  // IPv4 (4 bytes)
  if (key === 'ip' && buf.length === 4) {
    return Array.from(buf).join('.');
  }
  // Public key — show compressed hex
  if (key === 'secp256k1') {
    return buf.toString('hex');
  }
  // Capabilities / tags — RLP list
  if (key === 'a.caps' || key === 'a.tags') {
    try {
      const items = rlpDecode(buf) as Buffer[];
      if (Array.isArray(items)) {
        return items.map(b => b.toString('utf8')).join(', ');
      }
    } catch { /* fall through */ }
  }
  // Default: try UTF-8, fallback to hex
  try {
    const str = buf.toString('utf8');
    if (/^[\x20-\x7E]*$/.test(str)) return str;
  } catch { /* */ }
  return buf.toString('hex');
}

// ── agents list ───────────────────────────────────────────────────────────────

async function inspectAgents(options: InspectOptions): Promise<void> {
  const host = options.host ?? 'localhost';
  const port = options.port ?? '3000';
  const url  = `http://${host}:${port}/agents`;

  logger.title('Mesh Agents');
  logger.info(`Querying: ${url}`);
  console.log('');

  let agents: AgentRecord[];
  try {
    agents = JSON.parse(await httpGet(url));
  } catch (e) {
    logger.error(`Failed to reach discovery layer: ${e}`);
    logger.dim(`Is a discovery server running at ${host}:${port}?`);
    process.exit(1);
  }

  if (!agents.length) {
    logger.warn('No agents registered in the discovery layer.');
    return;
  }

  if (options.raw) {
    console.log(JSON.stringify(agents, null, 2));
    return;
  }

  const statusIcon = (s: string) => s === 'healthy' ? '✔' : s === 'degraded' ? '⚠' : '✘';

  for (const a of agents) {
    const icon = statusIcon(a.health?.status ?? 'unknown');
    console.log(`  ${icon}  ${a.name ?? a.agentId}`);
    console.log(`     ID:           ${a.agentId}`);
    console.log(`     Owner:        ${a.owner}`);
    console.log(`     Capabilities: ${(a.capabilities ?? []).join(', ')}`);
    console.log(`     Endpoint:     ${a.network?.protocol ?? 'http'}://${a.network?.host}:${a.network?.port}`);
    console.log(`     Health:       ${a.health?.status ?? 'unknown'}`);
    if (a.health?.lastHeartbeat) {
      const ago = Math.round((Date.now() - new Date(a.health.lastHeartbeat).getTime()) / 1000);
      console.log(`     Last seen:    ${ago}s ago`);
    }
    console.log('');
  }

  console.log(`  Total: ${agents.length} agent(s)`);
}

// ── single agent inspect ──────────────────────────────────────────────────────

async function inspectAgent(agentId: string, options: InspectOptions): Promise<void> {
  const host = options.host ?? 'localhost';
  const port = options.port ?? '8080';

  logger.title(`Inspecting agent: ${agentId}`);
  console.log('');

  // Fetch health, ANR, and capabilities in parallel
  const [healthRaw, anrRaw, capsRaw] = await Promise.allSettled([
    httpGet(`http://${host}:${port}/health`),
    httpGet(`http://${host}:${port}/anr`),
    httpGet(`http://${host}:${port}/capabilities`),
  ]);

  if (options.raw) {
    console.log(JSON.stringify({
      health:       healthRaw.status === 'fulfilled' ? JSON.parse(healthRaw.value) : null,
      anr:          anrRaw.status    === 'fulfilled' ? JSON.parse(anrRaw.value)    : null,
      capabilities: capsRaw.status   === 'fulfilled' ? JSON.parse(capsRaw.value)   : null,
    }, null, 2));
    return;
  }

  if (healthRaw.status === 'fulfilled') {
    try {
      const h = JSON.parse(healthRaw.value) as HealthResponse;
      console.log(`  Status:       ${h.status}`);
      console.log(`  Agent ID:     ${h.agentId ?? '—'}`);
      console.log(`  Version:      ${h.version ?? '—'}`);
      console.log(`  Uptime:       ${h.uptimeMs != null ? `${Math.round(h.uptimeMs / 1000)}s` : '—'}`);
      console.log(`  Capabilities: ${h.capabilitiesCount ?? '—'}`);
    } catch { /* */ }
  } else {
    logger.warn(`Health endpoint unreachable at http://${host}:${port}/health`);
  }

  console.log('');

  if (capsRaw.status === 'fulfilled') {
    try {
      const caps = JSON.parse(capsRaw.value) as { capabilities: CapSummary[] };
      console.log('  Capabilities:');
      for (const cap of caps.capabilities ?? []) {
        const price = cap.price ? `  [${cap.price}]` : '';
        console.log(`    • ${cap.name}${price}`);
        if (cap.description) console.log(`      ${cap.description}`);
      }
    } catch { /* */ }
  }

  console.log('');

  if (anrRaw.status === 'fulfilled') {
    try {
      const anr = JSON.parse(anrRaw.value);
      if (anr.anr) {
        console.log('  ANR text:');
        console.log(`    ${anr.anr}`);
        console.log('');
        console.log(`  Decode with: borgkit inspect anr "${anr.anr}"`);
      }
    } catch { /* */ }
  }
}

// ── capabilities list ─────────────────────────────────────────────────────────

async function inspectCapabilities(options: InspectOptions): Promise<void> {
  const host = options.host ?? 'localhost';
  const port = options.port ?? '3000';
  const url  = `http://${host}:${port}/agents`;

  logger.title('All Mesh Capabilities');
  logger.info(`Querying: ${url}`);
  console.log('');

  let agents: AgentRecord[];
  try {
    agents = JSON.parse(await httpGet(url));
  } catch (e) {
    logger.error(`Failed to reach discovery layer: ${e}`);
    process.exit(1);
  }

  // Build capability → agents map
  const capMap = new Map<string, string[]>();
  for (const a of agents) {
    for (const cap of a.capabilities ?? []) {
      if (!capMap.has(cap)) capMap.set(cap, []);
      capMap.get(cap)!.push(a.name ?? a.agentId);
    }
  }

  if (capMap.size === 0) {
    logger.warn('No capabilities found.');
    return;
  }

  if (options.raw) {
    const out: Record<string, string[]> = {};
    for (const [cap, providers] of capMap) out[cap] = providers;
    console.log(JSON.stringify(out, null, 2));
    return;
  }

  const sorted = Array.from(capMap.entries()).sort(([a], [b]) => a.localeCompare(b));
  for (const [cap, providers] of sorted) {
    const reserved = cap.startsWith('__') ? ' (reserved)' : '';
    console.log(`  ${cap}${reserved}`);
    for (const p of providers) console.log(`    ↳ ${p}`);
  }

  console.log('');
  console.log(`  Total: ${capMap.size} unique capability(-ies) across ${agents.length} agent(s)`);
}

// ── minimal RLP decoder ───────────────────────────────────────────────────────

function rlpDecode(data: Buffer): unknown {
  const [result] = rlpDecodeAt(data, 0);
  return result;
}

function rlpDecodeAt(data: Buffer, offset: number): [unknown, number] {
  const prefix = data[offset];
  if (prefix < 0x80) {
    return [data.subarray(offset, offset + 1), offset + 1];
  }
  if (prefix <= 0xb7) {
    const len   = prefix - 0x80;
    const start = offset + 1;
    return [data.subarray(start, start + len), start + len];
  }
  if (prefix <= 0xbf) {
    const lenLen = prefix - 0xb7;
    const len    = readBigEndian(data, offset + 1, lenLen);
    const start  = offset + 1 + lenLen;
    return [data.subarray(start, start + len), start + len];
  }
  if (prefix <= 0xf7) {
    const payloadLen = prefix - 0xc0;
    const items: unknown[] = [];
    let   pos   = offset + 1;
    const end   = pos + payloadLen;
    while (pos < end) {
      const [item, next] = rlpDecodeAt(data, pos);
      items.push(item);
      pos = next;
    }
    return [items, end];
  }
  const lenLen     = prefix - 0xf7;
  const payloadLen = readBigEndian(data, offset + 1, lenLen);
  const items: unknown[] = [];
  let   pos = offset + 1 + lenLen;
  const end = pos + payloadLen;
  while (pos < end) {
    const [item, next] = rlpDecodeAt(data, pos);
    items.push(item);
    pos = next;
  }
  return [items, end];
}

function readBigEndian(buf: Buffer, offset: number, len: number): number {
  let n = 0;
  for (let i = 0; i < len; i++) n = (n * 256) + buf[offset + i];
  return n;
}

function readUint64BE(buf: Buffer): number {
  if (buf.length < 8) return 0;
  // JS can handle up to 2^53 safely
  return buf.readUInt32BE(0) * 0x100000000 + buf.readUInt32BE(4);
}

function formatRlpForJson(v: unknown): unknown {
  if (Buffer.isBuffer(v)) return v.toString('hex');
  if (Array.isArray(v))   return v.map(formatRlpForJson);
  return v;
}

// ── HTTP helper ───────────────────────────────────────────────────────────────

function httpGet(url: string): Promise<string> {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let d = '';
      res.on('data', c => { d += c; });
      res.on('end', () => resolve(d));
    }).on('error', reject);
  });
}

// ── types ─────────────────────────────────────────────────────────────────────

interface AgentRecord {
  agentId:      string;
  name?:        string;
  owner:        string;
  capabilities: string[];
  network?:     { protocol: string; host: string; port: number };
  health?:      { status: string; lastHeartbeat?: string };
}

interface HealthResponse {
  status:            string;
  agentId?:          string;
  version?:          string;
  uptimeMs?:         number;
  capabilitiesCount?: number;
}

interface CapSummary {
  name:         string;
  description?: string;
  price?:       string;
}
