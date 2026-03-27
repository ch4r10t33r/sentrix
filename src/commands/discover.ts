import { logger } from '../utils/logger';
import http from 'http';

interface DiscoverOptions {
  capability?: string;
  host: string;
  port: string;
}

export async function discoverCommand(options: DiscoverOptions): Promise<void> {
  const { capability, host, port } = options;
  const query = capability ? `?capability=${encodeURIComponent(capability)}` : '';
  const url = `http://${host}:${port}/agents${query}`;

  logger.title('Borgkit Discovery Query');
  logger.info(`Querying: ${url}`);

  try {
    const data = await httpGet(url);
    const agents: AgentSummary[] = JSON.parse(data);

    if (!agents.length) {
      logger.warn('No agents found matching the query.');
      return;
    }

    logger.success(`Found ${agents.length} agent(s):\n`);
    for (const agent of agents) {
      console.log(`  ID:           ${agent.agentId}`);
      console.log(`  Name:         ${agent.name}`);
      console.log(`  Owner:        ${agent.owner}`);
      console.log(`  Capabilities: ${agent.capabilities.join(', ')}`);
      console.log(`  Network:      ${agent.network.protocol}://${agent.network.host}:${agent.network.port}`);
      console.log(`  Health:       ${agent.health.status}`);
      console.log('');
    }
  } catch (err) {
    logger.error(`Discovery request failed: ${err}`);
    logger.dim(`Is the discovery layer running at ${host}:${port}?`);
    process.exit(1);
  }
}

interface AgentSummary {
  agentId: string;
  name: string;
  owner: string;
  capabilities: string[];
  network: { protocol: string; host: string; port: number };
  health: { status: string };
}

function httpGet(url: string): Promise<string> {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let data = '';
      res.on('data', chunk => { data += chunk; });
      res.on('end', () => resolve(data));
    }).on('error', reject);
  });
}
