use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::process::Command;

const NPM_PACKAGE: &str = "@ch4r10teer41/inai-cli";
const CURRENT_VERSION: &str = env!("INAI_VERSION");

pub fn run() -> Result<()> {
    println!("Checking for updates…");

    let output = Command::new("npm")
        .args(["view", NPM_PACKAGE, "version", "--json"])
        .output()
        .context("Failed to run `npm view` — is Node.js installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("npm registry lookup failed: {}", stderr.trim());
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let latest = raw.trim().trim_matches('"');

    let current = CURRENT_VERSION.trim_start_matches('v');
    let latest_clean = latest.trim_start_matches('v');

    println!(
        "  {:20} {}",
        "Current version:".bright_black(),
        current.yellow()
    );
    println!(
        "  {:20} {}",
        "Latest version:".bright_black(),
        latest_clean.green()
    );

    if current == latest_clean {
        println!("\n{}", "✓ Already up to date.".green().bold());
    } else {
        println!(
            "\n{} Run:\n\n  npm install -g {}@{}\n",
            "Update available!".yellow().bold(),
            NPM_PACKAGE,
            latest_clean,
        );
    }

    Ok(())
}
