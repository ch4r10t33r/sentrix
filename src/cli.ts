import { Command } from 'commander';
import { initCommand }     from './commands/init';
import { createCommand }   from './commands/create';
import { runCommand }      from './commands/run';
import { discoverCommand } from './commands/discover';
import { versionCommand }  from './commands/version';
import { testCommand }    from './commands/test';
import { inspectCommand } from './commands/inspect';
import { VERSION }         from './version';

const program = new Command();

program
  .name('borgkit')
  .description('Borgkit CLI — Autonomous Agentic Coordination Middleware: scaffold P2P-discoverable, DID-native agents across any framework')
  .version(VERSION, '-v, --version', 'Print the current Borgkit CLI version');

program
  .command('init <project-name>')
  .description('Scaffold a new Borgkit agent project')
  .option('-l, --lang <language>', 'Target language: typescript | python | rust | zig', 'typescript')
  .option('--no-discovery', 'Skip discovery adapter scaffolding')
  .option('--no-example', 'Skip example agent generation')
  .action(initCommand);

program
  .command('create agent <agent-name>')
  .description('Generate a new agent inside an existing Borgkit project')
  .option('-l, --lang <language>', 'Target language (auto-detected from project if omitted)')
  .option('-c, --capabilities <caps>', 'Comma-separated list of capability names', 'exampleCapability')
  .option('-f, --framework <framework>', 'Agent framework: none | google-adk | crewai | langgraph | agno | llamaindex | smolagents', 'none')
  .option('--addon <addon>', 'Optional add-on: x402')
  .option('-y, --yes', 'Skip confirmation prompts and auto-install dependencies')
  .action(createCommand);

program
  .command('run <agent-name>')
  .description('Start an agent in dev mode')
  .option('-p, --port <port>', 'Port to listen on', '8080')
  .option('--transport <transport>', 'Transport: http | websocket | grpc', 'http')
  .action(runCommand);

program
  .command('discover')
  .description('Query the local or remote discovery layer for agents by capability')
  .option('-c, --capability <cap>', 'Capability to search for')
  .option('--host <host>', 'Discovery host', 'localhost')
  .option('--port <port>', 'Discovery port', '3000')
  .action(discoverCommand);

program
  .command('version')
  .description('Show detailed version and build info')
  .action(versionCommand);

program
  .command('test [agent-name]')
  .description('Run unit tests for agents, or scaffold a test file')
  .option('--generate', 'Generate a test scaffold for the named agent')
  .option('--watch',    'Watch mode (TypeScript/Python only)')
  .option('--coverage', 'Collect coverage report')
  .action(testCommand);

program
  .command('inspect [subcommand] [target]')
  .description('Inspect ANR records and mesh topology')
  .option('--host <host>', 'Discovery or agent host', 'localhost')
  .option('--port <port>', 'Discovery or agent port', '3000')
  .option('--raw',         'Output raw JSON')
  .action(inspectCommand);

program.parse(process.argv);
