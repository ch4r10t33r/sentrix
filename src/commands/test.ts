/**
 * borgkit test — run unit tests for Borgkit agents.
 *
 * Usage:
 *   borgkit test                    # runs all tests in tests/
 *   borgkit test MyAgent            # runs tests/MyAgent.test.ts (or .py/.rs/.zig)
 *   borgkit test --generate MyAgent # scaffold a test file for MyAgent
 *
 * Test scaffold (TypeScript) tests:
 *   - Agent instantiation
 *   - get_capabilities() returns non-empty array
 *   - handle_request() returns valid AgentResponse for each capability
 *   - health check (status: "healthy")
 *   - ANR structure validation
 */

import path     from 'path';
import fs       from 'fs-extra';
import { spawn } from 'child_process';
import { detectLanguage }  from '../utils/detect-lang';
import { logger }          from '../utils/logger';

interface TestOptions {
  generate?: boolean;
  watch?:    boolean;
  coverage?: boolean;
}

export async function testCommand(
  agentName: string | undefined,
  options:   TestOptions,
): Promise<void> {
  const projectDir = process.cwd();
  const lang       = detectLanguage(projectDir);

  if (options.generate) {
    if (!agentName) {
      logger.error('--generate requires an agent name: borgkit test --generate MyAgent');
      process.exit(1);
    }
    await generateTestFile(projectDir, lang, agentName);
    return;
  }

  logger.title(`Running Borgkit tests (${lang})`);
  await runTests(projectDir, lang, agentName, options);
}

// ── test file generation ──────────────────────────────────────────────────────

async function generateTestFile(
  projectDir: string,
  lang:       string,
  agentName:  string,
): Promise<void> {
  const testsDir = path.join(projectDir, 'tests');
  await fs.ensureDir(testsDir);

  const generators: Record<string, () => { file: string; content: string }> = {
    typescript: () => ({
      file:    path.join(testsDir, `${agentName}.test.ts`),
      content: tsTestTemplate(agentName),
    }),
    python: () => {
      const snake = toSnakeCase(agentName);
      return {
        file:    path.join(testsDir, `test_${snake}.py`),
        content: pyTestTemplate(agentName, snake),
      };
    },
    rust: () => ({
      // Rust tests live in the agent source file (idiomatic)
      file:    path.join(projectDir, 'src', 'tests', `${toSnakeCase(agentName)}_test.rs`),
      content: rustTestTemplate(agentName),
    }),
    zig: () => ({
      file:    path.join(testsDir, `${toSnakeCase(agentName)}_test.zig`),
      content: zigTestTemplate(agentName),
    }),
  };

  const gen = generators[lang];
  if (!gen) {
    logger.error(`No test generator for language: ${lang}`);
    process.exit(1);
  }

  const { file, content } = gen();

  if (await fs.pathExists(file)) {
    logger.warn(`Test file already exists: ${path.relative(projectDir, file)}`);
    return;
  }

  await fs.outputFile(file, content);
  logger.success(`Generated test file: ${path.relative(projectDir, file)}`);
  logger.dim(`Run it with: borgkit test ${agentName}`);
}

// ── test runner dispatch ──────────────────────────────────────────────────────

async function runTests(
  projectDir: string,
  lang:       string,
  agentName:  string | undefined,
  options:    TestOptions,
): Promise<void> {
  type RunnerSpec = { cmd: string; args: string[]; env?: NodeJS.ProcessEnv };

  const runners: Record<string, () => RunnerSpec> = {
    typescript: () => {
      const pattern = agentName
        ? `tests/${agentName}.test.ts`
        : 'tests/**/*.test.ts';
      const args = ['jest', '--testPathPattern', pattern, '--passWithNoTests'];
      if (options.watch)    args.push('--watch');
      if (options.coverage) args.push('--coverage');
      return { cmd: 'npx', args };
    },
    python: () => {
      const args = ['-m', 'pytest', '-v'];
      if (agentName) args.push(`tests/test_${toSnakeCase(agentName)}.py`);
      else           args.push('tests/');
      if (options.coverage) args.push('--cov=agents', '--cov-report=term-missing');
      return { cmd: 'python', args };
    },
    rust: () => ({
      cmd:  'cargo',
      args: ['test', ...(agentName ? [`${toSnakeCase(agentName)}`] : [])],
    }),
    zig: () => ({
      cmd:  'zig',
      args: ['build', 'test'],
    }),
  };

  const runnerFn = runners[lang];
  if (!runnerFn) {
    logger.error(`No test runner for language: ${lang}`);
    process.exit(1);
  }

  const { cmd, args, env } = runnerFn();
  logger.info(`Running: ${cmd} ${args.join(' ')}`);

  const proc = spawn(cmd, args, {
    stdio:  'inherit',
    cwd:    projectDir,
    env:    env ?? process.env,
  });

  proc.on('error', (err) => {
    logger.error(`Failed to start test runner: ${err.message}`);
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      const hints: Record<string, string> = {
        typescript: 'Install Jest: npm install -D jest ts-jest @types/jest',
        python:     'Install pytest: pip install pytest pytest-asyncio pytest-cov',
        rust:       'Tests are built into cargo: cargo test',
        zig:        'Tests use zig build: zig build test',
      };
      if (hints[lang]) logger.info(hints[lang]);
    }
    process.exit(1);
  });

  await new Promise<void>((resolve, reject) => {
    proc.on('exit', (code) => {
      if (code === 0) {
        logger.success('All tests passed ✔');
        resolve();
      } else {
        logger.error(`Tests failed with exit code ${code}`);
        process.exit(code ?? 1);
      }
    });
    proc.on('error', reject);
  });
}

// ── test templates ────────────────────────────────────────────────────────────

function tsTestTemplate(agentName: string): string {
  return `/**
 * Unit tests for ${agentName}
 * Generated by: borgkit test --generate ${agentName}
 */
import { ${agentName} } from '../agents/${agentName}';

describe('${agentName}', () => {
  let agent: ${agentName};

  beforeEach(() => {
    agent = new ${agentName}();
  });

  // ── identity ──────────────────────────────────────────────────────────────

  test('agentId is a non-empty string', () => {
    expect(typeof agent.agentId()).toBe('string');
    expect(agent.agentId().length).toBeGreaterThan(0);
  });

  test('owner is a non-empty string', () => {
    expect(typeof agent.owner()).toBe('string');
    expect(agent.owner().length).toBeGreaterThan(0);
  });

  // ── capabilities ──────────────────────────────────────────────────────────

  test('getCapabilities returns a non-empty array', () => {
    const caps = agent.getCapabilities();
    expect(Array.isArray(caps)).toBe(true);
    expect(caps.length).toBeGreaterThan(0);
  });

  test('each capability name is a non-empty string', () => {
    for (const cap of agent.getCapabilities()) {
      expect(typeof cap).toBe('string');
      expect(cap.length).toBeGreaterThan(0);
    }
  });

  // ── request handling ──────────────────────────────────────────────────────

  test.each(
    // Dynamically derive test cases from declared capabilities
    (new ${agentName}()).getCapabilities().map(cap => [cap])
  )('handleRequest responds to capability: %s', async (capability) => {
    const req = {
      requestId:  'test-req-001',
      from:       'borgkit://test',
      capability,
      payload:    {},
      timestamp:  Date.now(),
    };
    const resp = await agent.handleRequest(req as any);

    expect(resp).toBeDefined();
    expect(typeof resp.requestId).toBe('string');
    expect(['success', 'error']).toContain(resp.status);
    expect(typeof resp.timestamp).toBe('number');
  });

  test('handleRequest returns error for unknown capability', async () => {
    const req = {
      requestId:  'test-req-unknown',
      from:       'borgkit://test',
      capability: '__unknown_capability__',
      payload:    {},
      timestamp:  Date.now(),
    };
    const resp = await agent.handleRequest(req as any);
    expect(resp.status).toBe('error');
  });

  // ── ANR ───────────────────────────────────────────────────────────────────

  test('getAnr returns a valid DiscoveryEntry', () => {
    const anr = agent.getAnr();
    expect(typeof anr.agentId).toBe('string');
    expect(typeof anr.name).toBe('string');
    expect(Array.isArray(anr.capabilities)).toBe(true);
    expect(typeof anr.network.host).toBe('string');
    expect(typeof anr.network.port).toBe('number');
  });

  // ── heartbeat ─────────────────────────────────────────────────────────────

  test('responds to __heartbeat capability', async () => {
    const req = {
      requestId:  'test-hb-001',
      from:       'borgkit://test',
      capability: '__heartbeat',
      payload:    { senderId: 'borgkit://test', timestamp: Date.now() },
      timestamp:  Date.now(),
    };
    const resp = await agent.handleRequest(req as any);
    expect(resp.status).toBe('success');
    expect(resp.result).toBeDefined();
  });
});
`;
}

function pyTestTemplate(agentName: string, snakeName: string): string {
  return `"""
Unit tests for ${agentName}
Generated by: borgkit test --generate ${agentName}
"""
import asyncio
import pytest
from agents.${snakeName} import ${agentName}
from interfaces.agent_request import AgentRequest
from interfaces.agent_response import AgentResponse


@pytest.fixture
def agent():
    return ${agentName}()


# ── identity ──────────────────────────────────────────────────────────────────

def test_agent_id_is_nonempty(agent):
    assert isinstance(agent.agent_id(), str)
    assert len(agent.agent_id()) > 0


def test_owner_is_nonempty(agent):
    assert isinstance(agent.owner(), str)
    assert len(agent.owner()) > 0


# ── capabilities ──────────────────────────────────────────────────────────────

def test_get_capabilities_returns_list(agent):
    caps = agent.get_capabilities()
    assert isinstance(caps, list)
    assert len(caps) > 0


def test_capability_names_are_strings(agent):
    for cap in agent.get_capabilities():
        assert isinstance(cap, str)
        assert len(cap) > 0


# ── request handling ──────────────────────────────────────────────────────────

@pytest.mark.asyncio
@pytest.mark.parametrize("capability", ${agentName}().get_capabilities())
async def test_handle_request_per_capability(agent, capability):
    req = AgentRequest(
        request_id="test-req-001",
        from_id="borgkit://test",
        capability=capability,
        payload={},
        timestamp=0,
    )
    resp = await agent.handle_request(req)
    assert resp is not None
    assert resp.request_id == "test-req-001"
    assert resp.status in ("success", "error")


@pytest.mark.asyncio
async def test_unknown_capability_returns_error(agent):
    req = AgentRequest(
        request_id="test-req-unknown",
        from_id="borgkit://test",
        capability="__unknown_capability__",
        payload={},
        timestamp=0,
    )
    resp = await agent.handle_request(req)
    assert resp.status == "error"


# ── ANR ───────────────────────────────────────────────────────────────────────

def test_get_anr_returns_valid_entry(agent):
    entry = agent.get_anr()
    assert hasattr(entry, "agent_id")
    assert hasattr(entry, "capabilities")
    assert isinstance(entry.capabilities, list)


# ── heartbeat ─────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_heartbeat_capability(agent):
    req = AgentRequest(
        request_id="test-hb-001",
        from_id="borgkit://test",
        capability="__heartbeat",
        payload={"senderId": "borgkit://test", "timestamp": 0},
        timestamp=0,
    )
    resp = await agent.handle_request(req)
    assert resp.status == "success"
    assert resp.result is not None
`;
}

function rustTestTemplate(agentName: string): string {
  const snake = toSnakeCase(agentName);
  return `/// Unit tests for ${agentName}
/// Generated by: borgkit test --generate ${agentName}
///
/// Run with: cargo test ${snake}

#[cfg(test)]
mod ${snake}_tests {
    use super::*;
    use crate::request::AgentRequest;
    use serde_json::json;

    fn make_agent() -> ${agentName} {
        ${agentName}::new()
    }

    fn make_request(capability: &str) -> AgentRequest {
        AgentRequest {
            request_id:  "test-req-001".to_string(),
            from:        "borgkit://test".to_string(),
            capability:  capability.to_string(),
            payload:     json!({}),
            signature:   None,
            timestamp:   Some(0),
            session_key: None,
            payment:     None,
        }
    }

    // ── identity ──────────────────────────────────────────────────────────

    #[test]
    fn agent_id_is_nonempty() {
        let agent = make_agent();
        assert!(!agent.agent_id().is_empty());
    }

    #[test]
    fn owner_is_nonempty() {
        let agent = make_agent();
        assert!(!agent.owner().is_empty());
    }

    // ── capabilities ──────────────────────────────────────────────────────

    #[test]
    fn get_capabilities_returns_nonempty() {
        let agent = make_agent();
        let caps  = agent.get_capabilities();
        assert!(!caps.is_empty(), "agent must declare at least one capability");
    }

    // ── request handling ──────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_request_each_capability() {
        let agent = make_agent();
        for cap in agent.get_capabilities() {
            let req  = make_request(&cap);
            let resp = agent.handle_request(req).await;
            assert!(
                resp.status == "success" || resp.status == "error",
                "unexpected status '{}' for capability '{}'", resp.status, cap
            );
        }
    }

    #[tokio::test]
    async fn unknown_capability_returns_error() {
        let agent = make_agent();
        let req   = make_request("__unknown_capability__");
        let resp  = agent.handle_request(req).await;
        assert_eq!(resp.status, "error");
    }

    // ── ANR ───────────────────────────────────────────────────────────────

    #[test]
    fn get_anr_has_required_fields() {
        let agent = make_agent();
        let entry = agent.get_anr();
        assert!(!entry.agent_id.is_empty());
        assert!(!entry.capabilities.is_empty());
    }
}
`;
}

function zigTestTemplate(agentName: string): string {
  const snake = toSnakeCase(agentName);
  return `//! Unit tests for ${agentName}
//! Generated by: borgkit test --generate ${agentName}
//!
//! Run with: zig build test

const std   = @import("std");
const types = @import("../src/types.zig");
const agent_mod = @import("../src/${snake}.zig");

const Agent = agent_mod.${agentName};

test "${agentName} — agentId is non-empty" {
    var agent = Agent{};
    try std.testing.expect(agent.agentId().len > 0);
}

test "${agentName} — owner is non-empty" {
    var agent = Agent{};
    try std.testing.expect(agent.owner().len > 0);
}

test "${agentName} — getCapabilities returns non-empty slice" {
    var agent = Agent{};
    const caps = agent.getCapabilities();
    try std.testing.expect(caps.len > 0);
}

test "${agentName} — handleRequest echo capability" {
    var agent = Agent{};
    const caps = agent.getCapabilities();
    if (caps.len == 0) return;

    const req = types.AgentRequest{
        .request_id = "test-req-001",
        .from       = "borgkit://test",
        .capability = caps[0],
        .payload    = "{}",
        .timestamp  = 0,
    };

    const resp = agent.handleRequest(req);
    try std.testing.expect(
        std.mem.eql(u8, resp.status, "success") or
        std.mem.eql(u8, resp.status, "error")
    );
    try std.testing.expectEqualStrings("test-req-001", resp.request_id);
}

test "${agentName} — unknown capability returns error" {
    var agent = Agent{};
    const req = types.AgentRequest{
        .request_id = "test-req-unknown",
        .from       = "borgkit://test",
        .capability = "__unknown_capability__",
        .payload    = "{}",
        .timestamp  = 0,
    };
    const resp = agent.handleRequest(req);
    try std.testing.expectEqualStrings("error", resp.status);
}
`;
}

// ── helpers ───────────────────────────────────────────────────────────────────

function toSnakeCase(s: string): string {
  return s.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '');
}
