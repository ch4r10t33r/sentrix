import path   from 'path';
import fs     from 'fs-extra';
import { detectLanguage } from '../utils/detect-lang';
import { logger }         from '../utils/logger';
import { spawn }          from 'child_process';

interface RunOptions {
  port:      string;
  transport: string;
}

export async function runCommand(agentName: string, options: RunOptions): Promise<void> {
  const projectDir = process.cwd();
  const lang       = detectLanguage(projectDir);
  const port       = options.port ?? '8080';

  logger.title(`Starting ${agentName} on ${options.transport}://localhost:${port}`);

  // ── resolve the agent file ──────────────────────────────────────────────────

  function snakeCase(s: string): string {
    return s.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '');
  }

  const agentFiles: Record<string, string[]> = {
    typescript: [
      path.join(projectDir, 'agents', `${agentName}.ts`),
    ],
    python: [
      path.join(projectDir, 'agents', `${snakeCase(agentName)}.py`),
      path.join(projectDir, 'agents', `${agentName}.py`),
    ],
    rust: [],   // handled below via cargo
    zig:  [],   // handled below via zig build
  };

  // Verify agent file exists for file-based languages
  if (lang === 'typescript' || lang === 'python') {
    const candidates = agentFiles[lang] ?? [];
    const found = candidates.find(f => fs.pathExistsSync(f));
    if (!found) {
      logger.error(
        `Agent file not found. Looked for:\n` +
        candidates.map(f => `  ${f}`).join('\n')
      );
      process.exit(1);
    }
  }

  // ── build runner command ────────────────────────────────────────────────────

  const runners: Record<string, () => { cmd: string; args: string[]; env?: NodeJS.ProcessEnv }> = {

    typescript: () => ({
      cmd: 'npx',
      args: [
        'ts-node',
        '--project', 'tsconfig.json',
        path.join('agents', `${agentName}.ts`),
      ],
      env: { ...process.env, BORGKIT_PORT: port },
    }),

    python: () => {
      // Determine snake_case filename
      const snakeName = snakeCase(agentName);
      const fileExists = fs.pathExistsSync(
        path.join(projectDir, 'agents', `${snakeName}.py`)
      );
      const moduleName = fileExists ? snakeName : agentName;
      return {
        cmd: 'python',
        args: [
          path.join('agents', `${moduleName}.py`),
        ],
        env: { ...process.env, BORGKIT_PORT: port },
      };
    },

    rust: () => ({
      cmd: 'cargo',
      args: ['run', '--', agentName, '--port', port],
      env: { ...process.env, BORGKIT_PORT: port },
    }),

    zig: () => ({
      cmd: 'zig',
      args: ['build', 'run', '--', agentName, '--port', port],
      env: { ...process.env, BORGKIT_PORT: port },
    }),
  };

  const runnerFn = runners[lang];
  if (!runnerFn) {
    logger.error(`No runner configured for language "${lang}".`);
    process.exit(1);
  }

  const { cmd, args, env } = runnerFn();

  logger.info(`Running: ${cmd} ${args.join(' ')}`);

  const proc = spawn(cmd, args, {
    stdio: 'inherit',
    cwd:   projectDir,
    env,
  });

  proc.on('error', (err) => {
    logger.error(`Failed to start agent: ${err.message}`);
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      const hints: Record<string, string> = {
        typescript: 'Make sure Node.js ≥ 20 and ts-node are installed: npm install -D ts-node',
        python:     'Make sure Python 3.11+ is installed and in PATH',
        rust:       'Make sure the Rust toolchain is installed: https://rustup.rs',
        zig:        'Make sure Zig 0.12+ is installed: https://ziglang.org/download',
      };
      if (hints[lang]) logger.info(hints[lang]);
    }
    process.exit(1);
  });

  proc.on('exit', (code) => {
    if (code !== 0) {
      logger.error(`Agent exited with code ${code}`);
      process.exit(code ?? 1);
    }
  });
}
