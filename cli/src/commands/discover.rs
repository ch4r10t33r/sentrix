use anyhow::{Context, Result};
use clap::Args;
use owo_colors::OwoColorize;
use serde::Deserialize;

use crate::logger;

#[derive(Args)]
pub struct DiscoverArgs {
    #[arg(short, long)]
    pub capability: Option<String>,
    #[arg(long, default_value = "localhost")]
    pub host: String,
    #[arg(short, long, default_value = "6174")]
    pub port: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSummary {
    agent_id: String,
    name: String,
    capabilities: Vec<String>,
    #[serde(default)]
    health: Option<HealthInfo>,
}

#[derive(Deserialize)]
struct HealthInfo {
    status: String,
}

pub fn run(args: DiscoverArgs) -> Result<()> {
    let mut url = format!("http://{}:{}/agents", args.host, args.port);
    if let Some(ref cap) = args.capability {
        url.push_str(&format!("?capability={}", cap));
    }

    logger::title("Borgkit Discovery Query");
    logger::info(&format!("Querying: {}", url));

    let response = reqwest::blocking::get(&url).context("Could not reach discovery server")?;

    // Handle both array `[...]` and object `{"agents": [...]}` shapes.
    let raw: serde_json::Value = response
        .json()
        .context("Failed to parse response as JSON")?;

    let agents: Vec<AgentSummary> = if raw.is_array() {
        serde_json::from_value(raw).context("Failed to deserialize agent list")?
    } else {
        let arr = raw
            .get("agents")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value(arr).context("Failed to deserialize agent list")?
    };

    if agents.is_empty() {
        logger::warn("No agents found");
        return Ok(());
    }

    for agent in &agents {
        println!();
        // Name (bold) and ID (dimmed) on the same line.
        println!(
            "  {} {}",
            agent.name.bold(),
            format!("({})", agent.agent_id).dimmed()
        );

        // Health status icon.
        let health_line = match &agent.health {
            Some(h) => match h.status.as_str() {
                "healthy" => format!("{} healthy", "✔".green()),
                "degraded" => format!("{} degraded", "⚠".yellow()),
                other => format!("{} {}", "✖".red(), other.red()),
            },
            None => format!("{} unknown", "✖".red()),
        };
        println!("  {:20} {}", "Health:".bright_black(), health_line);

        // Capabilities as a comma-joined cyan list.
        println!(
            "  {:20} {}",
            "Capabilities:".bright_black(),
            agent.capabilities.join(", ").cyan()
        );
    }

    println!();
    logger::success(&format!("Found {} agent(s)", agents.len()));

    Ok(())
}
