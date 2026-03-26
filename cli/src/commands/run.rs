use anyhow::{Context, Result};
use clap::Args;
use std::process::Stdio;

use crate::detect_lang;
use crate::logger;

#[derive(Args)]
pub struct RunArgs {
    /// Agent name to run
    pub agent: String,
    #[arg(short, long, default_value = "6174")]
    pub port: u16,
    #[arg(long, default_value = "http")]
    pub transport: String,
    #[arg(short, long)]
    pub lang: Option<String>,
}

/// Convert PascalCase / camelCase to snake_case.
/// e.g. "MyAgent" → "my_agent", "myAgent" → "my_agent"
fn snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.char_indices() {
        if ch.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.push(ch.to_lowercase().next().unwrap());
    }
    out
}

pub fn run(args: RunArgs) -> Result<()> {
    // 1. Detect language
    let lang = detect_lang::detect(args.lang.as_deref(), &std::env::current_dir()?)?;

    // 2. Build runner command
    let agent_snake = snake_case(&args.agent);
    let port_str = args.port.to_string();

    let ts_agent_path = format!("agents/{}.ts", args.agent);
    let py_agent_path = format!("agents/{}.py", agent_snake);

    let (cmd, runner_args): (&str, Vec<&str>) = match lang {
        detect_lang::Lang::TypeScript => (
            "npx",
            vec!["ts-node", "--project", "tsconfig.json", &ts_agent_path],
        ),
        detect_lang::Lang::Python => ("python", vec![&py_agent_path]),
        detect_lang::Lang::Rust => ("cargo", vec!["run", "--", &args.agent, "--port", &port_str]),
        detect_lang::Lang::Zig => (
            "zig",
            vec!["build", "run", "--", &args.agent, "--port", &port_str],
        ),
    };

    // 3. Print start banner
    logger::title(&format!(
        "Starting {} on {}://localhost:{}",
        args.agent, args.transport, args.port
    ));
    logger::kv("Agent", &args.agent);
    logger::kv("Port", &port_str);
    logger::kv("Transport", &args.transport);
    logger::kv("Language", &lang.to_string());
    logger::dim("Press Ctrl+C to stop");

    // 4. Spawn child process
    let status = std::process::Command::new(cmd)
        .args(&runner_args)
        .env("SENTRIX_PORT", args.port.to_string())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to start '{}'. Is it installed?", cmd))?;

    std::process::exit(status.code().unwrap_or(1));
}
