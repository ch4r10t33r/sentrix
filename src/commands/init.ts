import path from 'path';
import fs from 'fs-extra';
import ora from 'ora';
import { copyTemplate, writeConfig } from '../utils/generator';
import { logger } from '../utils/logger';

interface InitOptions {
  lang: string;
  discovery: boolean;
  example: boolean;
}

export async function initCommand(
  projectName: string,
  options: InitOptions
): Promise<void> {
  const { lang, discovery, example } = options;
  const supportedLangs = ['typescript', 'python', 'rust', 'zig'];

  if (!supportedLangs.includes(lang)) {
    logger.error(`Unsupported language "${lang}". Choose from: ${supportedLangs.join(', ')}`);
    process.exit(1);
  }

  const projectDir = path.join(process.cwd(), projectName);

  if (fs.existsSync(projectDir)) {
    logger.error(`Directory "${projectName}" already exists.`);
    process.exit(1);
  }

  logger.title(`Borgkit — Initialising project "${projectName}" [${lang}]`);

  const spinner = ora('Scaffolding project structure...').start();

  try {
    const templateRoot = path.join(__dirname, '../../templates', lang);

    // Core interfaces
    await copyTemplate(
      path.join(templateRoot, 'interfaces'),
      path.join(projectDir, 'interfaces'),
      { PROJECT_NAME: projectName }
    );

    // Example agent (optional)
    if (example) {
      await copyTemplate(
        path.join(templateRoot, 'agents'),
        path.join(projectDir, 'agents'),
        { PROJECT_NAME: projectName, AGENT_NAME: 'ExampleAgent', CAPABILITIES: 'exampleCapability' }
      );
    }

    // Discovery adapter (optional)
    if (discovery) {
      await copyTemplate(
        path.join(templateRoot, 'discovery'),
        path.join(projectDir, 'discovery'),
        { PROJECT_NAME: projectName }
      );
    }

    // Language-specific root files (package.json, tsconfig, Cargo.toml, etc.)
    const rootFiles = await fs.readdir(templateRoot);
    for (const file of rootFiles) {
      const srcPath = path.join(templateRoot, file);
      const stat = await fs.stat(srcPath);
      if (stat.isFile()) {
        const destName = file.endsWith('.tpl') ? file.slice(0, -4) : file;
        await fs.copy(srcPath, path.join(projectDir, destName));
      }
    }

    // Write borgkit.config.json
    await writeConfig(projectDir, {
      projectName,
      lang,
      version: '0.1.0',
      discovery: discovery ? { adapter: 'local', host: 'localhost', port: 3000 } : false,
      network: { protocol: 'http', port: 8080, tls: false }
    });

    spinner.succeed('Project scaffolded successfully!');

    // Print the tree
    logger.title('Project layout:');
    logger.tree(`${projectName}/`);
    logger.tree('├── interfaces/        # ERC-8004 core interfaces');
    if (example)   logger.tree('├── agents/            # Agent implementations');
    if (discovery) logger.tree('├── discovery/         # Discovery adapter');
    logger.tree('└── borgkit.config.json # Project configuration');

    logger.title('Next steps:');
    logger.info(`  cd ${projectName}`);

    if (lang === 'typescript') {
      logger.info('  npm install');
      logger.info('  npm run dev');
    } else if (lang === 'python') {
      logger.info('  pip install -r requirements.txt');
      logger.info('  python -m agents.example_agent');
    } else if (lang === 'rust') {
      logger.info('  cargo build');
      logger.info('  cargo run');
    } else if (lang === 'zig') {
      logger.info('  zig build');
      logger.info('  zig build run');
    }

    logger.info(`\n  borgkit create agent <name>   # add more agents`);
    logger.info('  borgkit discover              # query discovery layer\n');
  } catch (err) {
    spinner.fail('Scaffolding failed.');
    logger.error(String(err));
    process.exit(1);
  }
}
