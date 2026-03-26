use anyhow::{anyhow, Context, Result};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::detect_lang;
use crate::logger;
use crate::templates::Templates;

#[derive(Args)]
pub struct InitArgs {
    /// Project directory name
    pub name: String,
    /// Language template: typescript, python, rust, zig
    #[arg(short, long, default_value = "typescript")]
    pub lang: String,
    /// Skip copying discovery files
    #[arg(long)]
    pub no_discovery: bool,
    /// Skip copying example agent
    #[arg(long)]
    pub no_example: bool,
}

/// Convert a hyphen/underscore-separated string to PascalCase.
/// e.g. "my-cool_agent" → "MyCoolAgent"
fn pascal_case(s: &str) -> String {
    s.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// Format a UNIX timestamp (seconds) as a minimal ISO 8601 UTC string.
/// e.g. "2026-03-26T12:34:56Z"
fn unix_to_iso8601(secs: u64) -> String {
    // Days since epoch → broken-down date via the proleptic Gregorian calendar.
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    // Compute year/month/day from days-since-epoch (algorithm by Henry S. Warren Jr.)
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

pub fn run(args: InitArgs) -> Result<()> {
    // 1. Validate project name
    if args.name.is_empty() {
        return Err(anyhow!("Project name must not be empty."));
    }
    if args.name.contains('/') || args.name.contains('\\') {
        return Err(anyhow!(
            "Project name '{}' must not contain path separators.",
            args.name
        ));
    }

    // 2. Parse language
    let lang = detect_lang::Lang::from_str(&args.lang).ok_or_else(|| {
        anyhow!(
            "Unknown language '{}'. Valid: typescript, python, rust, zig",
            args.lang
        )
    })?;

    // 3. Create project directory
    let project_path: PathBuf = std::env::current_dir()?.join(&args.name);

    if project_path.exists() {
        // Error if non-empty
        let is_empty = project_path
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(anyhow!(
                "Directory '{}' already exists and is non-empty.",
                args.name
            ));
        }
    } else {
        std::fs::create_dir_all(&project_path)
            .with_context(|| format!("Failed to create directory '{}'", args.name))?;
    }

    let agent_name = pascal_case(&args.name);
    let lang_str = lang.template_dir();
    let prefix = format!("{}/", lang_str);

    // 4. Spinner
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message(format!("Scaffolding {} project…", lang_str));
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));

    // 5. Extract template files
    let mut created_files: Vec<String> = Vec::new();

    for key in Templates::iter() {
        let key: &str = &key;

        if !key.starts_with(&prefix) {
            continue;
        }

        // Strip the "<lang>/" prefix
        let rel = &key[prefix.len()..];

        // Skip discovery/ if --no-discovery
        if args.no_discovery && rel.starts_with("discovery/") {
            continue;
        }

        // Skip agents/ if --no-example
        if args.no_example && rel.starts_with("agents/") {
            continue;
        }

        // Strip .tpl suffix from the destination filename
        let rel_dest = if let Some(stripped) = rel.strip_suffix(".tpl") {
            stripped.to_string()
        } else {
            rel.to_string()
        };

        // Get file content
        let embedded = Templates::get(key).unwrap();
        let raw = std::str::from_utf8(&embedded.data)
            .with_context(|| format!("Template '{}' is not valid UTF-8", key))?;

        // Apply token replacement
        let content = raw
            .replace("{{PROJECT_NAME}}", &args.name)
            .replace("{{AGENT_NAME}}", &agent_name)
            .replace("{{CAPABILITIES}}", "echo, ping");

        // Destination path
        let dest = project_path.join(&rel_dest);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
        }

        // Write file
        let mut file = std::fs::File::create(&dest)
            .with_context(|| format!("Failed to create file '{}'", dest.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write file '{}'", dest.display()))?;

        // Preserve execute permissions for shell scripts on Unix
        #[cfg(unix)]
        if rel_dest.ends_with(".sh") {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&dest)?.permissions();
            let mode = perms.mode();
            perms.set_mode(mode | 0o111);
            std::fs::set_permissions(&dest, perms)?;
        }

        created_files.push(rel_dest);
    }

    // 6. Write sentrix.config.json
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let created_at = unix_to_iso8601(now_secs);

    let config = serde_json::json!({
        "language":  lang_str,
        "agentName": agent_name,
        "version":   "0.1.0",
        "port":      6174,
        "createdAt": created_at
    });

    let config_path = project_path.join("sentrix.config.json");
    let config_file =
        std::fs::File::create(&config_path).context("Failed to create sentrix.config.json")?;
    serde_json::to_writer_pretty(config_file, &config)
        .context("Failed to write sentrix.config.json")?;

    // 7. Print summary
    spinner.finish_and_clear();
    logger::success(&format!("Project '{}' scaffolded successfully!", args.name));

    logger::title(&format!("{}/", args.name));
    for f in &created_files {
        let (prefix_sym, name) = if f == created_files.last().unwrap() {
            ("└──", f.as_str())
        } else {
            ("├──", f.as_str())
        };
        logger::tree(prefix_sym, name);
    }
    logger::tree("└──", "sentrix.config.json");

    logger::title("Next steps:");
    logger::dim(&format!("  cd {} && sentrix run {}", args.name, agent_name));

    Ok(())
}
