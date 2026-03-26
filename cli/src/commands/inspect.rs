use anyhow::{anyhow, Context, Result};
use clap::Args;
use owo_colors::OwoColorize;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::logger;

#[derive(Args)]
pub struct InspectArgs {
    /// Subcommand: anr | agents | agent | capabilities
    pub subcommand: Option<String>,
    /// Target (ANR text, agent-id, etc.)
    pub target: Option<String>,
    #[arg(long, default_value = "localhost")]
    pub host: String,
    #[arg(short, long, default_value = "6174")]
    pub port: u16,
    /// Print raw JSON instead of formatted output
    #[arg(long)]
    pub raw: bool,
}

pub fn run(args: InspectArgs) -> Result<()> {
    match args.subcommand.as_deref() {
        Some("anr") => {
            let text = args
                .target
                .ok_or_else(|| anyhow!("Usage: sentrix inspect anr <anr-text>"))?;
            inspect_anr(&text, args.raw)
        }
        Some("agents") => inspect_agents(&args.host, args.port, args.raw),
        Some("agent") => {
            let id = args
                .target
                .ok_or_else(|| anyhow!("Usage: sentrix inspect agent <agent-id>"))?;
            inspect_agent(&id, &args.host, args.port, args.raw)
        }
        Some("capabilities") => inspect_capabilities(&args.host, args.port, args.raw),
        Some(other) => Err(anyhow!(
            "Unknown subcommand '{other}'. Try: anr | agents | agent | capabilities"
        )),
        None => {
            println!("\n{}", "sentrix inspect — subcommands".bold());
            println!();
            println!(
                "  {}  Decode an ANR record",
                "sentrix inspect anr <anr-text>  ".cyan()
            );
            println!(
                "  {}  List all mesh agents",
                "sentrix inspect agents           ".cyan()
            );
            println!(
                "  {}  Inspect one agent",
                "sentrix inspect agent <agent-id> ".cyan()
            );
            println!(
                "  {}  All capabilities on the mesh",
                "sentrix inspect capabilities     ".cyan()
            );
            println!();
            println!(
                "  {} --host <host>   discovery/agent host  (default: localhost)",
                "Options:".bright_black()
            );
            println!(
                "  {} --port <port>   discovery/agent port  (default: 6174)",
                "        ".bright_black()
            );
            println!(
                "  {} --raw           print raw JSON",
                "        ".bright_black()
            );
            Ok(())
        }
    }
}

// ── ANR decoder ───────────────────────────────────────────────────────────────

const ANR_PREFIX: &str = "anr:";

fn anr_key_names(key: &str) -> &str {
    match key {
        "id" => "id-scheme",
        "secp256k1" => "public-key (secp256k1)",
        "ip" => "IPv4",
        "ip6" => "IPv6",
        "tcp" => "TCP port",
        "udp" => "UDP port",
        "a.id" => "agent-id",
        "a.name" => "name",
        "a.ver" => "version",
        "a.caps" => "capabilities",
        "a.tags" => "tags",
        "a.proto" => "protocol",
        "a.port" => "agent port",
        "a.tls" => "TLS",
        "a.meta" => "metadata URI",
        "a.owner" => "owner",
        "a.chain" => "chain ID",
        other => other,
    }
}

fn inspect_anr(text: &str, raw: bool) -> Result<()> {
    let text = text.trim();
    if !text.starts_with(ANR_PREFIX) {
        return Err(anyhow!("ANR text must start with \"anr:\""));
    }
    let b64 = &text[ANR_PREFIX.len()..];

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64)
        .context("Failed to decode ANR base64url")?;

    let items = rlp_decode_list(&bytes).ok_or_else(|| anyhow!("ANR decode: expected RLP list"))?;

    if items.len() < 2 {
        return Err(anyhow!("ANR decode: expected at least [sig, seq, ...kv]"));
    }

    if raw {
        let hex_items: Vec<String> = items.iter().map(|b| hex(b)).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "raw":   hex(&bytes),
                "items": hex_items,
            }))?
        );
        return Ok(());
    }

    println!("\n{}", "ANR Record".bold());
    println!();

    let sig_hex = hex(&items[0]);
    println!(
        "  {:<22} {}…",
        "signature".bright_black(),
        &sig_hex[..sig_hex.len().min(32)]
    );

    let seq = read_u64_be(&items[1]);
    println!("  {:<22} {seq}", "sequence".bright_black());
    println!();

    let kv = &items[2..];
    let mut i = 0;
    while i + 1 < kv.len() {
        let key_str = std::str::from_utf8(&kv[i]).unwrap_or("?").to_string();
        let label = anr_key_names(&key_str);
        let display = format_anr_value(&key_str, &kv[i + 1]);
        println!("  {:<22} {}", label.bright_black(), display);
        i += 2;
    }

    println!();
    println!(
        "  {:<22} {} / 512 bytes",
        "size".bright_black(),
        bytes.len()
    );
    if bytes.len() > 400 {
        logger::warn(&format!(
            "Record is {} bytes — approaching 512-byte limit",
            bytes.len()
        ));
    }
    Ok(())
}

fn format_anr_value(key: &str, buf: &[u8]) -> String {
    match key {
        "tcp" | "udp" | "a.port" => {
            if buf.len() >= 2 {
                let port = u16::from_be_bytes([buf[0], buf[1]]);
                port.to_string()
            } else {
                hex(buf)
            }
        }
        "a.chain" => {
            if buf.len() >= 8 {
                read_u64_be(buf).to_string()
            } else {
                hex(buf)
            }
        }
        "a.tls" => {
            if buf.first() == Some(&1) {
                "true".into()
            } else {
                "false".into()
            }
        }
        "ip" if buf.len() == 4 => {
            format!("{}.{}.{}.{}", buf[0], buf[1], buf[2], buf[3])
        }
        "secp256k1" => hex(buf),
        "a.caps" | "a.tags" => {
            if let Some(items) = rlp_decode_list(buf) {
                items
                    .iter()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                hex(buf)
            }
        }
        _ => {
            if let Ok(s) = std::str::from_utf8(buf) {
                if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                    return s.to_string();
                }
            }
            hex(buf)
        }
    }
}

// ── minimal self-contained RLP decoder ───────────────────────────────────────

/// Decode an RLP-encoded outer list, returning its elements as byte slices.
fn rlp_decode_list(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if data.is_empty() {
        return None;
    }
    let prefix = data[0] as usize;
    let (payload_start, payload_len) = if prefix <= 0xf7 {
        (1, prefix - 0xc0)
    } else {
        let ll = prefix - 0xf7;
        if data.len() < 1 + ll {
            return None;
        }
        let len = read_be_uint(&data[1..1 + ll]);
        (1 + ll, len)
    };
    if data.len() < payload_start + payload_len {
        return None;
    }

    let mut items = Vec::new();
    let mut pos = payload_start;
    let end = payload_start + payload_len;
    while pos < end {
        let (item, next) = rlp_item_at(data, pos)?;
        items.push(item.to_vec());
        pos = next;
    }
    Some(items)
}

fn rlp_item_at(data: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    if offset >= data.len() {
        return None;
    }
    let prefix = data[offset] as usize;
    if prefix < 0x80 {
        return Some((&data[offset..offset + 1], offset + 1));
    }
    if prefix <= 0xb7 {
        let len = prefix - 0x80;
        let start = offset + 1;
        return Some((&data[start..start + len], start + len));
    }
    if prefix <= 0xbf {
        let ll = prefix - 0xb7;
        let len = read_be_uint(&data[offset + 1..offset + 1 + ll]);
        let start = offset + 1 + ll;
        return Some((&data[start..start + len], start + len));
    }
    // list — return entire encoded list as opaque bytes
    let (payload_start, payload_len) = if prefix <= 0xf7 {
        (offset + 1, prefix - 0xc0)
    } else {
        let ll = prefix - 0xf7;
        let len = read_be_uint(&data[offset + 1..offset + 1 + ll]);
        (offset + 1 + ll, len)
    };
    let end = payload_start + payload_len;
    Some((&data[offset..end], end))
}

fn read_be_uint(b: &[u8]) -> usize {
    b.iter().fold(0usize, |acc, &x| (acc << 8) | x as usize)
}

fn read_u64_be(b: &[u8]) -> u64 {
    b.iter().take(8).fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ── agents list ───────────────────────────────────────────────────────────────

#[derive(Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AgentRecord {
    agent_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    network: NetworkInfo,
    #[serde(default)]
    health: HealthInfo,
}

#[derive(Deserialize, serde::Serialize, Default)]
struct NetworkInfo {
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
}

#[derive(Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct HealthInfo {
    #[serde(default)]
    status: String,
    #[serde(default)]
    last_heartbeat: String,
}

fn fetch_agents(host: &str, port: u16) -> Result<Vec<AgentRecord>> {
    let url = format!("http://{host}:{port}/agents");
    let body: serde_json::Value = reqwest::blocking::get(&url)
        .with_context(|| format!("Could not reach {url}"))?
        .json()?;
    let arr = if body.is_array() {
        body
    } else {
        body.get("agents")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]))
    };
    Ok(serde_json::from_value(arr)?)
}

fn health_icon(status: &str) -> &'static str {
    match status {
        "healthy" => "✔",
        "degraded" => "⚠",
        _ => "✘",
    }
}

fn inspect_agents(host: &str, port: u16, raw: bool) -> Result<()> {
    println!("\n{}", "Mesh Agents".bold());
    logger::info(&format!("Querying: http://{host}:{port}/agents"));
    println!();

    let agents = fetch_agents(host, port)?;
    if agents.is_empty() {
        logger::warn("No agents registered.");
        return Ok(());
    }

    if raw {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::to_value(&agents)?)?
        );
        return Ok(());
    }

    for a in &agents {
        let icon = health_icon(&a.health.status);
        println!("  {}  {}", icon.green(), a.name.bold());
        println!("  {:<20} {}", "ID:".bright_black(), a.agent_id.dimmed());
        if !a.owner.is_empty() {
            println!("  {:<20} {}", "Owner:".bright_black(), &a.owner);
        }
        if !a.capabilities.is_empty() {
            println!(
                "  {:<20} {}",
                "Capabilities:".bright_black(),
                a.capabilities.join(", ").cyan()
            );
        }
        if !a.network.host.is_empty() {
            println!(
                "  {:<20} {}://{}:{}",
                "Endpoint:".bright_black(),
                a.network.protocol,
                a.network.host,
                a.network.port
            );
        }
        if !a.health.last_heartbeat.is_empty() {
            println!(
                "  {:<20} {}",
                "Last heartbeat:".bright_black(),
                &a.health.last_heartbeat
            );
        }
        println!();
    }
    println!("  Total: {} agent(s)", agents.len().to_string().green());
    Ok(())
}

// ── single agent ─────────────────────────────────────────────────────────────

fn inspect_agent(id: &str, host: &str, port: u16, raw: bool) -> Result<()> {
    println!("\n{} {}", "Inspecting:".bold(), id.cyan());
    println!();

    // Fetch /health, /anr, /capabilities in parallel threads
    let base = format!("http://{host}:{port}");
    let (tx1, rx1) = std::sync::mpsc::channel();
    let (tx2, rx2) = std::sync::mpsc::channel();
    let (tx3, rx3) = std::sync::mpsc::channel();

    let b1 = base.clone();
    std::thread::spawn(move || {
        tx1.send(
            reqwest::blocking::get(format!("{b1}/health"))
                .and_then(|r| r.text())
                .ok(),
        )
        .ok();
    });
    let b2 = base.clone();
    std::thread::spawn(move || {
        tx2.send(
            reqwest::blocking::get(format!("{b2}/anr"))
                .and_then(|r| r.text())
                .ok(),
        )
        .ok();
    });
    let b3 = base.clone();
    std::thread::spawn(move || {
        tx3.send(
            reqwest::blocking::get(format!("{b3}/capabilities"))
                .and_then(|r| r.text())
                .ok(),
        )
        .ok();
    });

    let health_raw = rx1.recv().ok().flatten();
    let anr_raw = rx2.recv().ok().flatten();
    let caps_raw = rx3.recv().ok().flatten();

    if raw {
        let parse =
            |s: Option<String>| s.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "health":       parse(health_raw),
                "anr":          parse(anr_raw),
                "capabilities": parse(caps_raw),
            }))?
        );
        return Ok(());
    }

    if let Some(h) = health_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
    {
        println!(
            "  {:<22} {}",
            "Status:".bright_black(),
            h["status"].as_str().unwrap_or("—")
        );
        println!(
            "  {:<22} {}",
            "Agent ID:".bright_black(),
            h["agentId"].as_str().unwrap_or("—")
        );
        if let Some(v) = h["version"].as_str() {
            println!("  {:<22} {}", "Version:".bright_black(), v);
        }
        if let Some(u) = h["uptimeMs"].as_u64() {
            println!("  {:<22} {}s", "Uptime:".bright_black(), u / 1000);
        }
        if let Some(n) = h["capabilitiesCount"].as_u64() {
            println!("  {:<22} {n}", "Capabilities:".bright_black());
        }
    } else {
        logger::warn(&format!(
            "Health endpoint unreachable at http://{host}:{port}/health"
        ));
    }
    println!();

    if let Some(c) = caps_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
    {
        if let Some(caps) = c["capabilities"].as_array() {
            println!("  {}", "Capabilities:".bold());
            for cap in caps {
                let name = cap["name"].as_str().unwrap_or(cap.as_str().unwrap_or("?"));
                let price = cap["price"]
                    .as_str()
                    .map(|p| format!("  [{}]", p))
                    .unwrap_or_default();
                println!("    • {}{}", name.cyan(), price);
                if let Some(d) = cap["description"].as_str() {
                    println!("      {}", d.dimmed());
                }
            }
            println!();
        }
    }

    if let Some(a) = anr_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
    {
        if let Some(anr_text) = a["anr"].as_str() {
            println!("  {}", "ANR:".bold());
            println!("    {}", anr_text.dimmed());
            println!();
            println!("  Decode with: sentrix inspect anr \"{}\"", anr_text);
        }
    }
    Ok(())
}

// ── capabilities map ──────────────────────────────────────────────────────────

fn inspect_capabilities(host: &str, port: u16, raw: bool) -> Result<()> {
    println!("\n{}", "All Mesh Capabilities".bold());
    logger::info(&format!("Querying: http://{host}:{port}/agents"));
    println!();

    let agents = fetch_agents(host, port)?;
    let mut cap_map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for a in &agents {
        for cap in &a.capabilities {
            cap_map
                .entry(cap.clone())
                .or_default()
                .push(if a.name.is_empty() {
                    a.agent_id.clone()
                } else {
                    a.name.clone()
                });
        }
    }

    if cap_map.is_empty() {
        logger::warn("No capabilities found.");
        return Ok(());
    }

    if raw {
        println!("{}", serde_json::to_string_pretty(&cap_map)?);
        return Ok(());
    }

    for (cap, providers) in &cap_map {
        let reserved = if cap.starts_with("__") {
            " (reserved)".dimmed().to_string()
        } else {
            String::new()
        };
        println!("  {}{}", cap.cyan(), reserved);
        for p in providers {
            println!("    {} {}", "↳".bright_black(), p);
        }
    }

    println!();
    println!(
        "  {} unique capability(-ies) across {} agent(s)",
        cap_map.len().to_string().green(),
        agents.len().to_string().green()
    );
    Ok(())
}
