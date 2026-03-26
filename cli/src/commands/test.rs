use anyhow::{Context, Result};
use clap::Args;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::Stdio;

use crate::detect_lang::{self, Lang};
use crate::logger;

#[derive(Args)]
pub struct TestArgs {
    /// Agent name (optional — runs all tests if omitted)
    pub agent: Option<String>,
    /// Scaffold a test file instead of running tests
    #[arg(long)]
    pub generate: bool,
    /// Language override
    #[arg(short, long)]
    pub lang: Option<String>,
    /// Watch mode (TypeScript only)
    #[arg(long)]
    pub watch: bool,
    /// Enable coverage (TypeScript and Python)
    #[arg(long)]
    pub coverage: bool,
}

pub fn run(args: TestArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let lang = detect_lang::detect(args.lang.as_deref(), &cwd)?;

    if args.generate {
        let name = args.agent.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "--generate requires an agent name: sentrix test --generate <AgentName>"
            )
        })?;
        return generate_test(&cwd, &lang, name);
    }

    run_tests(
        &cwd,
        &lang,
        args.agent.as_deref(),
        args.watch,
        args.coverage,
    )
}

// ── generate test scaffold ────────────────────────────────────────────────────

fn generate_test(project_dir: &Path, lang: &Lang, agent_name: &str) -> Result<()> {
    let tests_dir = project_dir.join("tests");
    std::fs::create_dir_all(&tests_dir)?;

    let (dest, content) = match lang {
        Lang::TypeScript => (
            tests_dir.join(format!("{agent_name}.test.ts")),
            ts_test_template(agent_name),
        ),
        Lang::Python => {
            let snake = snake_case(agent_name);
            (
                tests_dir.join(format!("test_{snake}.py")),
                py_test_template(agent_name, &snake),
            )
        }
        Lang::Rust => {
            let snake = snake_case(agent_name);
            let dir = project_dir.join("src").join("tests");
            std::fs::create_dir_all(&dir)?;
            (
                dir.join(format!("{snake}_test.rs")),
                rust_test_template(agent_name),
            )
        }
        Lang::Zig => {
            let snake = snake_case(agent_name);
            (
                tests_dir.join(format!("{snake}_test.zig")),
                zig_test_template(agent_name),
            )
        }
    };

    if dest.exists() {
        logger::warn(&format!("File already exists: {}", dest.display()));
    }
    std::fs::write(&dest, content)?;
    logger::success(&format!("Test scaffold written: {}", dest.display()));
    logger::dim(&format!("Run with: sentrix test {agent_name}"));
    Ok(())
}

// ── run tests ─────────────────────────────────────────────────────────────────

fn run_tests(
    project_dir: &Path,
    lang: &Lang,
    agent: Option<&str>,
    watch: bool,
    coverage: bool,
) -> Result<()> {
    let label = agent.unwrap_or("all");
    println!(
        "\n{}",
        format!("Running Sentrix tests ({lang}) — {label}").bold()
    );

    let (cmd, args) = build_runner(lang, agent, watch, coverage);
    logger::info(&format!("Running: {} {}", cmd, args.join(" ")));

    let status = std::process::Command::new(cmd)
        .args(&args)
        .current_dir(project_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to start test runner '{cmd}'. Is it installed?"))?;

    if status.success() {
        logger::success("All tests passed ✔");
        Ok(())
    } else {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn build_runner<'a>(
    lang: &Lang,
    agent: Option<&str>,
    watch: bool,
    coverage: bool,
) -> (&'a str, Vec<String>) {
    match lang {
        Lang::TypeScript => {
            let mut a = vec!["jest".to_string(), "--passWithNoTests".to_string()];
            if let Some(name) = agent {
                a.push("--testPathPattern".to_string());
                a.push(format!("tests/{name}.test.ts"));
            }
            if watch {
                a.push("--watch".to_string());
            }
            if coverage {
                a.push("--coverage".to_string());
            }
            ("npx", a)
        }
        Lang::Python => {
            let mut a = vec!["-m".to_string(), "pytest".to_string(), "-v".to_string()];
            if let Some(name) = agent {
                a.push(format!("tests/test_{}.py", snake_case(name)));
            }
            if coverage {
                a.push("--cov".to_string());
            }
            ("python", a)
        }
        Lang::Rust => {
            let mut a = vec!["test".to_string()];
            if let Some(name) = agent {
                a.push(snake_case(name));
            }
            ("cargo", a)
        }
        Lang::Zig => ("zig", vec!["build".to_string(), "test".to_string()]),
    }
}

// ── test templates ────────────────────────────────────────────────────────────

fn ts_test_template(name: &str) -> String {
    format!(
        r#"/**
 * Unit tests for {name}
 * Generated by: sentrix test --generate {name}
 */
import {{ {name} }} from '../agents/{name}';

describe('{name}', () => {{
  let agent: {name};

  beforeEach(() => {{
    agent = new {name}();
  }});

  test('agentId is a non-empty string', () => {{
    expect(typeof agent.agentId).toBe('string');
    expect(agent.agentId.length).toBeGreaterThan(0);
  }});

  test('getCapabilities returns a non-empty array', () => {{
    const caps = agent.getCapabilities();
    expect(Array.isArray(caps)).toBe(true);
    expect(caps.length).toBeGreaterThan(0);
  }});

  test.each(
    (new {name}()).getCapabilities().map(cap => [cap])
  )('handleRequest responds to capability: %s', async (capability) => {{
    const req = {{
      requestId:  'test-req-001',
      from:       'sentrix://test',
      capability,
      payload:    {{}},
      timestamp:  Date.now(),
    }};
    const resp = await agent.handleRequest(req as any);
    expect(resp).toBeDefined();
    expect(['success', 'error']).toContain(resp.status);
  }});

  test('handleRequest returns error for unknown capability', async () => {{
    const req = {{
      requestId:  'test-req-unknown',
      from:       'sentrix://test',
      capability: '__unknown_capability__',
      payload:    {{}},
      timestamp:  Date.now(),
    }};
    const resp = await agent.handleRequest(req as any);
    expect(resp.status).toBe('error');
  }});

  test('getAnr returns a valid DiscoveryEntry', () => {{
    const anr = agent.getAnr();
    expect(typeof anr.agentId).toBe('string');
    expect(Array.isArray(anr.capabilities)).toBe(true);
    expect(typeof anr.network.port).toBe('number');
  }});
}});
"#
    )
}

fn py_test_template(name: &str, snake: &str) -> String {
    format!(
        r#""""
Unit tests for {name}
Generated by: sentrix test --generate {name}
"""
import asyncio
import pytest
from agents.{snake} import {name}
from interfaces import AgentRequest


@pytest.fixture
def agent():
    return {name}()


def test_agent_id_nonempty(agent):
    assert len(agent.agent_id) > 0


def test_get_capabilities(agent):
    caps = agent.get_capabilities()
    assert isinstance(caps, list) and len(caps) > 0


@pytest.mark.asyncio
@pytest.mark.parametrize("cap", {name}().get_capabilities())
async def test_handle_request(agent, cap):
    req = AgentRequest(request_id="t1", from_id="sentrix://test",
                       capability=cap, payload={{}})
    resp = await agent.handle_request(req)
    assert resp.status in ("success", "error")


@pytest.mark.asyncio
async def test_unknown_capability(agent):
    req = AgentRequest(request_id="t2", from_id="sentrix://test",
                       capability="__unknown__", payload={{}})
    resp = await agent.handle_request(req)
    assert resp.status == "error"


def test_get_anr(agent):
    entry = agent.get_anr()
    assert entry.agent_id
    assert isinstance(entry.capabilities, list)
"#
    )
}

fn rust_test_template(name: &str) -> String {
    let snake = snake_case(name);
    format!(
        r#"#[cfg(test)]
mod {snake}_tests {{
    use crate::request::AgentRequest;
    use crate::example_agent::{name};
    use serde_json::json;

    fn req(capability: &str) -> AgentRequest {{
        AgentRequest {{
            request_id:  "test-req".into(),
            from:        "sentrix://test".into(),
            capability:  capability.into(),
            payload:     json!({{}}),
            signature:   None,
            timestamp:   Some(0),
            session_key: None,
            payment:     None,
        }}
    }}

    #[test]
    fn agent_id_nonempty() {{
        let a = {name}::new();
        assert!(!a.agent_id.is_empty());
    }}

    #[test]
    fn capabilities_nonempty() {{
        let a = {name}::new();
        assert!(!a.get_capabilities().is_empty());
    }}

    #[tokio::test]
    async fn handles_each_capability() {{
        let a = {name}::new();
        for cap in a.get_capabilities() {{
            let r = a.handle_request(req(&cap)).await;
            assert!(r.status == "success" || r.status == "error");
        }}
    }}

    #[tokio::test]
    async fn unknown_capability_is_error() {{
        let a = {name}::new();
        let r = a.handle_request(req("__unknown__")).await;
        assert_eq!(r.status, "error");
    }}
}}
"#
    )
}

fn zig_test_template(name: &str) -> String {
    let snake = snake_case(name);
    format!(
        r#"const std   = @import("std");
const types = @import("../src/types.zig");
const mod   = @import("../src/{snake}.zig");

test "{name} — agentId non-empty" {{
    var a = mod.{name}{{}};
    try std.testing.expect(a.agentId().len > 0);
}}

test "{name} — capabilities non-empty" {{
    var a = mod.{name}{{}};
    try std.testing.expect(a.getCapabilities().len > 0);
}}

test "{name} — unknown capability returns error" {{
    var a = mod.{name}{{}};
    const req = types.AgentRequest{{
        .request_id = "t1",
        .from       = "sentrix://test",
        .capability = "__unknown__",
        .payload    = "{{}}",
        .timestamp  = 0,
    }};
    const resp = a.handleRequest(req);
    try std.testing.expectEqualStrings("error", resp.status);
}}
"#
    )
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_lowercase().next().unwrap());
    }
    out
}
