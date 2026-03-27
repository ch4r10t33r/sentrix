use anyhow::{anyhow, Context, Result};
use clap::Args;
use owo_colors::OwoColorize;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use crate::logger;

// ── Args ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ScaffoldArgs {
    /// Project / agent name (used as directory name and in generated code)
    pub name: String,

    /// Language: typescript | rust | zig
    #[arg(short, long, default_value = "typescript")]
    pub lang: String,

    /// Comma-separated plugins: openai,agno,langgraph,google_adk,crewai,llamaindex,smolagents,mcp
    #[arg(short, long, default_value = "")]
    pub plugins: String,

    /// Output directory  [default: current directory]
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Include DID key generation example
    #[arg(short, long)]
    pub did: bool,

    /// Include streaming (SSE) example
    #[arg(short, long)]
    pub stream: bool,

    /// Include x402 micropayments example
    #[arg(short = 'x', long)]
    pub x402: bool,

    /// Discovery backend: http | libp2p
    #[arg(long, default_value = "http")]
    pub discovery: String,

    /// Print what would be generated without writing files
    #[arg(long)]
    pub dry_run: bool,
}

// ── Language enum ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Lang {
    TypeScript,
    Rust,
    Zig,
}

impl Lang {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "typescript" | "ts" => Some(Self::TypeScript),
            "rust" | "rs" => Some(Self::Rust),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Rust => "rust",
            Self::Zig => "zig",
        }
    }
}

// ── Discovery enum ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Discovery {
    Http,
    Libp2p,
}

impl Discovery {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "http" => Some(Self::Http),
            "libp2p" => Some(Self::Libp2p),
            _ => None,
        }
    }
}

// ── A generated file: relative path + content ─────────────────────────────────

struct GenFile {
    rel_path: String,
    content: String,
}

impl GenFile {
    fn new(rel_path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            rel_path: rel_path.into(),
            content: content.into(),
        }
    }
}

// ── Plugin name normalisation ──────────────────────────────────────────────────

fn parse_plugins(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return vec![];
    }
    raw.split(',')
        .map(|p| p.trim().to_lowercase().replace(['-', '_'], ""))
        .filter(|p| !p.is_empty())
        .collect()
}

fn plugin_class_name(plugin: &str) -> &'static str {
    match plugin {
        "openai" => "OpenAIPlugin",
        "agno" => "AgnoPlugin",
        "langgraph" => "LangGraphPlugin",
        "googleadk" => "GoogleADKPlugin",
        "crewai" => "CrewAIPlugin",
        "llamaindex" => "LlamaIndexPlugin",
        "smolagents" => "SmolagentsPlugin",
        "mcp" => "MCPPlugin",
        _ => "UnknownPlugin",
    }
}

fn plugin_file_name(plugin: &str) -> String {
    format!("{}.ts", plugin_class_name(plugin))
}

/// Convert a hyphen/underscore-separated string to PascalCase.
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

// ── TypeScript generators ─────────────────────────────────────────────────────

fn gen_ts_package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "dev":   "ts-node src/index.ts",
    "build": "tsc",
    "start": "node dist/index.js"
  }},
  "dependencies": {{
    "sentrix-sdk": "^0.1.0",
    "express":     "^4.18.2",
    "dotenv":      "^16.4.5"
  }},
  "devDependencies": {{
    "typescript":  "^5.4.5",
    "@types/node": "^20.12.7",
    "ts-node":     "^10.9.2"
  }}
}}
"#,
        name = name
    )
}

fn gen_ts_tsconfig() -> String {
    r#"{
  "compilerOptions": {
    "target":           "ES2022",
    "module":           "CommonJS",
    "moduleResolution": "node",
    "outDir":           "./dist",
    "rootDir":          "./src",
    "strict":           true,
    "esModuleInterop":  true,
    "skipLibCheck":     true,
    "resolveJsonModule":true,
    "declaration":      true,
    "sourceMap":        true
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules", "dist"]
}
"#
    .to_string()
}

fn gen_ts_index(agent_class: &str) -> String {
    format!(
        r#"import 'dotenv/config';
import {{ {agent_class} }} from './agent';

async function main() {{
  const agent = new {agent_class}();
  const port  = parseInt(process.env['PORT'] ?? '6174', 10);
  await agent.start(port);
}}

main().catch(err => {{
  console.error('[fatal]', err);
  process.exit(1);
}});
"#,
        agent_class = agent_class
    )
}

fn gen_ts_agent(
    name: &str,
    agent_class: &str,
    plugins: &[String],
    discovery: &Discovery,
    did: bool,
    stream: bool,
    x402: bool,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Imports
    lines.push("import express, { Request, Response } from 'express';".into());
    lines.push("import crypto from 'crypto';".into());
    lines.push(String::new());

    // Plugin imports
    for plugin in plugins {
        let class = plugin_class_name(plugin);
        let file = format!("./plugins/{}", plugin_class_name(plugin));
        lines.push(format!("import {{ {} }} from '{}';", class, file));
    }
    if !plugins.is_empty() {
        lines.push(String::new());
    }

    if did {
        lines.push("// DID key generation example".into());
        lines.push("// Requires: npm install @noble/ed25519".into());
        lines.push("// import * as ed from '@noble/ed25519';".into());
        lines.push(String::new());
    }

    // Agent class
    lines.push(format!(
        r#"export class {agent_class} {{
  private readonly agentId: string;
  private readonly app = express();
  private plugins: unknown[] = [];"#,
        agent_class = agent_class
    ));

    if did {
        lines.push("  private didKey?: string;".into());
    }

    lines.push(String::new());
    lines.push("  constructor() {".into());
    lines.push(format!(
        "    this.agentId = `sentrix://agent/{name}/${{crypto.randomUUID()}}`;",
        name = name
    ));

    // Plugin init
    for plugin in plugins {
        let class = plugin_class_name(plugin);
        lines.push(format!(
            r#"    this.plugins.push(new {class}({{
      agentId:       this.agentId,
      name:          '{name}',
      version:       '0.1.0',
      discoveryType: '{disc_type}',
      discoveryUrl:  process.env['SENTRIX_DISCOVERY_URL'],
      discoveryKey:  process.env['SENTRIX_DISCOVERY_KEY'],
    }}));"#,
            class = class,
            name = name,
            disc_type = if *discovery == Discovery::Libp2p {
                "libp2p"
            } else {
                "http"
            },
        ));
    }

    if did {
        lines.push(String::new());
        lines.push("    // Generate a DID key on startup (ed25519)".into());
        lines.push("    // Uncomment after installing @noble/ed25519:".into());
        lines.push(
            "    // this.didKey = Buffer.from(ed.utils.randomPrivateKey()).toString('hex');".into(),
        );
        lines.push(
            "    // console.log(`[DID] agent key: did:key:z6Mk${this.didKey.slice(0, 8)}...`);"
                .into(),
        );
    }

    lines.push("  }".into());
    lines.push(String::new());

    // setupRoutes
    lines.push("  private setupRoutes(): void {".into());
    lines.push("    this.app.use(express.json());".into());
    lines.push(String::new());

    if x402 {
        lines.push("    // x402 micropayment middleware".into());
        lines.push("    // Requires: npm install x402-express".into());
        lines.push("    // import { x402Middleware } from 'x402-express';".into());
        lines.push("    // this.app.use('/invoke', x402Middleware({ amount: '0.001', currency: 'USDC' }));".into());
        lines.push(String::new());
    }

    lines.push("    // Health check".into());
    lines.push("    this.app.get('/health', (_req: Request, res: Response) => {".into());
    lines.push("      res.json({ status: 'ok', agentId: this.agentId });".into());
    lines.push("    });".into());
    lines.push(String::new());

    lines.push("    // Invoke endpoint".into());
    lines.push("    this.app.post('/invoke', async (req: Request, res: Response) => {".into());
    lines.push("      try {".into());
    lines.push("        const result = await this.processTask(req.body);".into());
    lines.push("        res.json(result);".into());
    lines.push("      } catch (err: unknown) {".into());
    lines.push("        const msg = err instanceof Error ? err.message : String(err);".into());
    lines.push("        res.status(500).json({ status: 'error', errorMessage: msg });".into());
    lines.push("      }".into());
    lines.push("    });".into());

    if stream {
        lines.push(String::new());
        lines.push("    // Streaming (SSE) invoke endpoint".into());
        lines.push(
            "    this.app.post('/invoke/stream', async (req: Request, res: Response) => {".into(),
        );
        lines.push("      res.setHeader('Content-Type', 'text/event-stream');".into());
        lines.push("      res.setHeader('Cache-Control', 'no-cache');".into());
        lines.push("      res.setHeader('Connection', 'keep-alive');".into());
        lines.push("      try {".into());
        lines.push("        const result = await this.processTask(req.body);".into());
        lines.push("        res.write(`data: ${JSON.stringify({ type: 'chunk', delta: JSON.stringify(result) })}\\n\\n`);".into());
        lines.push("        res.write(`data: ${JSON.stringify({ type: 'end' })}\\n\\n`);".into());
        lines.push("      } catch (err: unknown) {".into());
        lines.push("        const msg = err instanceof Error ? err.message : String(err);".into());
        lines.push(
            "        res.write(`data: ${JSON.stringify({ type: 'end', error: msg })}\\n\\n`);"
                .into(),
        );
        lines.push("      } finally {".into());
        lines.push("        res.end();".into());
        lines.push("      }".into());
        lines.push("    });".into());
    }

    lines.push("  }".into());
    lines.push(String::new());

    // processTask
    lines.push("  // eslint-disable-next-line @typescript-eslint/no-explicit-any".into());
    lines.push("  async processTask(payload: Record<string, unknown>): Promise<unknown> {".into());
    if plugins.is_empty() {
        lines.push("    // TODO: implement your agent logic here".into());
        lines.push("    const capability = String(payload['capability'] ?? 'echo');".into());
        lines.push("    if (capability === 'echo') return { status: 'success', result: { echo: payload } };".into());
        lines.push("    if (capability === 'ping') return { status: 'success', result: { pong: true, agentId: this.agentId } };".into());
        lines.push("    return { status: 'error', errorMessage: `Unknown capability: \"${capability}\"` };".into());
    } else {
        lines.push("    // Delegates to the first registered plugin".into());
        lines.push("    const plugin = this.plugins[0] as Record<string, Function>;".into());
        lines.push("    if (plugin && typeof plugin['invoke'] === 'function') {".into());
        lines.push("      return plugin['invoke'](payload);".into());
        lines.push("    }".into());
        lines.push("    return { status: 'error', errorMessage: 'No plugin available to handle request' };".into());
    }
    lines.push("  }".into());
    lines.push(String::new());

    // registerDiscovery
    let discovery_comment = if *discovery == Discovery::Libp2p {
        "// Libp2p discovery — requires @sentrix/libp2p-discovery"
    } else {
        "// HTTP discovery registry"
    };
    lines.push("  private async registerDiscovery(): Promise<void> {".into());
    lines.push(format!("    {}", discovery_comment));
    lines.push("    const url = process.env['SENTRIX_DISCOVERY_URL'];".into());
    lines.push("    if (!url) { console.warn('[discovery] SENTRIX_DISCOVERY_URL not set — skipping registration'); return; }".into());
    lines.push("    try {".into());
    lines.push("      const resp = await fetch(`${url}/agents`, {".into());
    lines.push("        method:  'POST',".into());
    lines.push("        headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${process.env['SENTRIX_DISCOVERY_KEY'] ?? ''}` },".into());
    lines.push(format!(
        "        body: JSON.stringify({{ agentId: this.agentId, name: '{name}', capabilities: ['{cap}'], network: {{ protocol: '{proto}', host: process.env['SENTRIX_HOST'] ?? 'localhost', port: parseInt(process.env['PORT'] ?? '6174', 10) }} }}),",
        name = name,
        cap  = name,
        proto = if *discovery == Discovery::Libp2p { "libp2p" } else { "http" },
    ));
    lines.push("      });".into());
    lines.push("      if (resp.ok) console.log(`[discovery] registered ${this.agentId}`);".into());
    lines
        .push("      else console.warn(`[discovery] registration failed: ${resp.status}`);".into());
    lines.push("    } catch (err) {".into());
    lines.push("      console.warn('[discovery] registration error:', err);".into());
    lines.push("    }".into());
    lines.push("  }".into());
    lines.push(String::new());

    // start
    lines.push("  async start(port = 6174): Promise<void> {".into());
    lines.push("    this.setupRoutes();".into());
    lines.push("    await new Promise<void>(resolve => {".into());
    lines.push("      this.app.listen(port, () => {".into());
    lines.push(format!(
        "        console.log(`[{name}] listening on http://localhost:${{port}}`);",
        name = name
    ));
    lines.push("        resolve();".into());
    lines.push("      });".into());
    lines.push("    });".into());
    lines.push("    await this.registerDiscovery();".into());
    lines.push(String::new());
    lines.push("    // Graceful shutdown".into());
    lines.push("    const shutdown = async () => { process.exit(0); };".into());
    lines.push("    process.on('SIGINT',  shutdown);".into());
    lines.push("    process.on('SIGTERM', shutdown);".into());
    lines.push("  }".into());

    lines.push("}".into());
    lines.push(String::new());

    lines.join("\n")
}

fn gen_ts_plugin_stub(plugin: &str) -> String {
    let class = plugin_class_name(plugin);
    format!(
        r#"/**
 * {class} stub — adapt this file to wire up the {plugin} framework.
 *
 * Install the framework SDK, then implement the methods below.
 * See templates/typescript/plugins/{class}.ts for the full implementation.
 */

export interface PluginConfig {{
  agentId:       string;
  name:          string;
  version:       string;
  discoveryType?: string;
  discoveryUrl?:  string;
  discoveryKey?:  string;
}}

export class {class} {{
  constructor(private readonly config: PluginConfig) {{}}

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  async invoke(payload: Record<string, unknown>): Promise<unknown> {{
    // TODO: implement {plugin} invocation
    console.log('[{class}] invoke called with', payload);
    return {{ status: 'success', result: {{ message: 'stub response from {plugin}' }} }};
  }}
}}
"#,
        class = class,
        plugin = plugin,
    )
}

fn gen_ts_env_example(name: &str, discovery: &Discovery) -> String {
    let disc_url = if *discovery == Discovery::Libp2p {
        "# SENTRIX_DISCOVERY_URL=  # not used in libp2p mode"
    } else {
        "SENTRIX_DISCOVERY_URL=http://localhost:8080"
    };
    format!(
        r#"# {name} — environment variables
# Copy to .env and fill in your values.

# Discovery
{disc_url}
SENTRIX_DISCOVERY_KEY=your-api-key-here

# Agent identity
SENTRIX_HOST=localhost
PORT=6174
SENTRIX_TLS=false

# Optional: ERC-8004 on-chain registry
# SENTRIX_REGISTRY_ADDRESS=0xYourContractAddress
# SENTRIX_RPC_URL=https://mainnet.infura.io/v3/YOUR_KEY

# Optional: agent signing key (32-byte hex, no 0x prefix)
# SENTRIX_AGENT_KEY=deadbeef...
"#,
        name = name,
        disc_url = disc_url,
    )
}

fn gen_ts_readme(name: &str, discovery: &Discovery, did: bool, stream: bool, x402: bool) -> String {
    let disc_note = if *discovery == Discovery::Libp2p {
        "This agent uses **libp2p** for peer-to-peer discovery."
    } else {
        "This agent uses **HTTP** discovery registry."
    };
    let mut extras = Vec::new();
    if did {
        extras.push("- **DID** key generation example included (`src/agent.ts`)");
    }
    if stream {
        extras.push("- **SSE streaming** available at `POST /invoke/stream`");
    }
    if x402 {
        extras.push("- **x402 micropayment** middleware stub included");
    }
    let extras_section = if extras.is_empty() {
        String::new()
    } else {
        format!("\n## Features\n\n{}\n", extras.join("\n"))
    };
    format!(
        r#"# {name}

A Sentrix P2P-discoverable agent scaffolded with `sentrix scaffold`.

{disc_note}
{extras_section}
## Prerequisites

- Node.js 18+
- npm 9+

## Quick start

```bash
cp .env.example .env
# Edit .env and set SENTRIX_DISCOVERY_URL, SENTRIX_DISCOVERY_KEY, etc.

npm install
npm run dev
```

## Invoke the agent

```bash
curl -s -X POST http://localhost:6174/invoke \
  -H 'Content-Type: application/json' \
  -d '{{"capability":"echo","payload":{{"hello":"world"}},"requestId":"req-1","from":"client"}}' | jq .
```

## Build for production

```bash
npm run build
npm start
```

## Project structure

```
{name}/
├── src/
│   ├── index.ts    — entry point
│   └── agent.ts    — agent implementation
├── .env.example
├── package.json
└── tsconfig.json
```
"#,
        name = name,
        disc_note = disc_note,
        extras_section = extras_section,
    )
}

// ── Rust generators ───────────────────────────────────────────────────────────

fn gen_rust_cargo_toml(name: &str) -> String {
    let lib_name = name.replace('-', "_");
    format!(
        r#"[package]
name    = "{lib_name}"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{lib_name}"
path = "src/main.rs"

[dependencies]
tokio   = {{ version = "1",    features = ["full"] }}
serde   = {{ version = "1",    features = ["derive"] }}
serde_json = "1"
reqwest = {{ version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }}
clap    = {{ version = "4",    features = ["derive"] }}
anyhow  = "1"
dotenvy = "0.15"
"#,
        lib_name = lib_name,
    )
}

fn gen_rust_main(agent_class: &str) -> String {
    format!(
        r#"mod agent;

use agent::{agent_class};

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    dotenvy::dotenv().ok();

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "6174".into())
        .parse()
        .unwrap_or(6174);

    let agent = {agent_class}::new();
    agent.start(port).await
}}
"#,
        agent_class = agent_class,
    )
}

fn gen_rust_agent(
    name: &str,
    agent_class: &str,
    plugins: &[String],
    discovery: &Discovery,
    did: bool,
    stream: bool,
    x402: bool,
) -> String {
    let disc_type = if *discovery == Discovery::Libp2p {
        "libp2p"
    } else {
        "http"
    };
    let mut lines: Vec<String> = vec![
        "use anyhow::Result;".into(),
        "use serde::{Deserialize, Serialize};".into(),
        "use serde_json::{json, Value};".into(),
        String::new(),
    ];

    if !plugins.is_empty() {
        lines.push("pub mod plugins;".into());
        lines.push(String::new());
    }

    if did {
        lines.push("// DID example: generate an ed25519 key pair".into());
        lines.push("// Requires: ed25519-dalek = \"2\"  in Cargo.toml".into());
        lines.push("// use ed25519_dalek::SigningKey;".into());
        lines.push(String::new());
    }

    lines.push("#[derive(Debug, Serialize, Deserialize)]".into());
    lines.push("pub struct InvokeRequest {".into());
    lines.push("    pub capability: String,".into());
    lines.push("    pub payload:    Value,".into());
    lines.push("    pub request_id: Option<String>,".into());
    lines.push("    pub from:       Option<String>,".into());
    lines.push("}".into());
    lines.push(String::new());

    lines.push("pub struct AgentInfo {".into());
    lines.push("    pub agent_id: String,".into());
    lines.push("    pub name:     &'static str,".into());
    lines.push("}".into());
    lines.push(String::new());

    lines.push(format!(
        "pub struct {agent_class} {{",
        agent_class = agent_class
    ));
    lines.push("    info: AgentInfo,".into());
    lines.push("}".into());
    lines.push(String::new());

    lines.push(format!("impl {agent_class} {{", agent_class = agent_class));
    lines.push("    pub fn new() -> Self {".into());
    lines.push("        Self {".into());
    lines.push("            info: AgentInfo {".into());
    lines.push(format!(
        "                agent_id: format!(\"sentrix://agent/{name}/{{uuid}}\", uuid = uuid_v4()),",
        name = name
    ));
    lines.push(format!(
        "                name:     \"{name}\",",
        name = name
    ));
    lines.push("            },".into());
    lines.push("        }".into());
    lines.push("    }".into());
    lines.push(String::new());

    lines.push("    pub async fn process_task(&self, req: &InvokeRequest) -> Value {".into());
    if plugins.is_empty() {
        lines.push("        match req.capability.as_str() {".into());
        lines.push("            \"echo\" => json!({ \"status\": \"success\", \"result\": { \"echo\": &req.payload } }),".into());
        lines.push("            \"ping\" => json!({ \"status\": \"success\", \"result\": { \"pong\": true, \"agentId\": &self.info.agent_id } }),".into());
        lines.push("            cap    => json!({ \"status\": \"error\", \"errorMessage\": format!(\"Unknown capability: {cap}\") }),".into());
        lines.push("        }".into());
    } else {
        lines.push("        // TODO: delegate to the appropriate plugin".into());
        lines.push(
            "        json!({ \"status\": \"success\", \"result\": { \"message\": \"stub\" } })"
                .into(),
        );
    }
    lines.push("    }".into());
    lines.push(String::new());

    lines.push("    async fn register_discovery(&self) {".into());
    lines.push(format!(
        "        let disc_type = \"{disc_type}\";",
        disc_type = disc_type
    ));
    lines.push("        let url = match std::env::var(\"SENTRIX_DISCOVERY_URL\") {".into());
    lines.push("            Ok(u) => u,".into());
    lines.push("            Err(_) => {".into());
    lines.push(
        "                eprintln!(\"[discovery] SENTRIX_DISCOVERY_URL not set — skipping\");"
            .into(),
    );
    lines.push("                return;".into());
    lines.push("            }".into());
    lines.push("        };".into());
    lines.push(
        "        let key  = std::env::var(\"SENTRIX_DISCOVERY_KEY\").unwrap_or_default();".into(),
    );
    lines.push("        let host = std::env::var(\"SENTRIX_HOST\").unwrap_or_else(|_| \"localhost\".into());".into());
    lines.push("        let port: u16 = std::env::var(\"PORT\").ok().and_then(|p| p.parse().ok()).unwrap_or(6174);".into());
    lines.push("        let body = json!({".into());
    lines.push("            \"agentId\":      &self.info.agent_id,".into());
    lines.push(format!(
        "            \"name\":         \"{name}\",",
        name = name
    ));
    lines.push(format!(
        "            \"capabilities\": [\"{name}\"],",
        name = name
    ));
    lines.push("            \"network\": {".into());
    lines.push("                \"protocol\": disc_type,".into());
    lines.push("                \"host\":     host,".into());
    lines.push("                \"port\":     port,".into());
    lines.push("            }".into());
    lines.push("        });".into());
    lines.push("        let client = reqwest::blocking::Client::new();".into());
    lines.push("        match client.post(format!(\"{}/agents\", url))".into());
    lines.push("            .header(\"Authorization\", format!(\"Bearer {}\", key))".into());
    lines.push("            .json(&body)".into());
    lines.push("            .send()".into());
    lines.push("        {".into());
    lines.push("            Ok(r) if r.status().is_success() =>".into());
    lines.push(
        "                println!(\"[discovery] registered {}\", self.info.agent_id),".into(),
    );
    lines.push("            Ok(r) =>".into());
    lines.push(
        "                eprintln!(\"[discovery] registration failed: {}\", r.status()),".into(),
    );
    lines.push("            Err(e) =>".into());
    lines.push("                eprintln!(\"[discovery] error: {e}\"),".into());
    lines.push("        }".into());
    lines.push("    }".into());
    lines.push(String::new());

    lines.push("    pub async fn start(self, port: u16) -> Result<()> {".into());
    lines.push("        use std::sync::Arc;".into());
    lines.push("        let me = Arc::new(self);".into());
    lines.push("        let me_disc = Arc::clone(&me);".into());
    lines.push(String::new());
    lines.push("        // Register with discovery in a blocking thread".into());
    lines.push("        tokio::task::spawn_blocking(move || {".into());
    lines.push(
        "            tokio::runtime::Handle::current().block_on(me_disc.register_discovery());"
            .into(),
    );
    lines.push("        });".into());
    lines.push(String::new());
    lines.push("        // Minimal HTTP server using tokio".into());
    lines.push("        use tokio::net::TcpListener;".into());
    lines.push("        use tokio::io::{AsyncReadExt, AsyncWriteExt};".into());
    lines.push(
        "        let listener = TcpListener::bind(format!(\"0.0.0.0:{}\", port)).await?;".into(),
    );
    lines.push(format!(
        "        println!(\"[{name}] listening on http://localhost:{{port}}\");",
        name = name
    ));
    lines.push("        loop {".into());
    lines.push("            let (mut socket, _) = listener.accept().await?;".into());
    lines.push("            let agent = Arc::clone(&me);".into());
    lines.push("            tokio::spawn(async move {".into());
    lines.push("                let mut buf = vec![0u8; 8192];".into());
    lines.push("                let n = socket.read(&mut buf).await.unwrap_or(0);".into());
    lines.push("                if n == 0 { return; }".into());
    lines.push("                let raw = String::from_utf8_lossy(&buf[..n]);".into());
    lines.push("                // naive body extraction".into());
    lines.push(
        "                let body_str = raw.splitn(2, \"\\r\\n\\r\\n\").nth(1).unwrap_or(\"\");"
            .into(),
    );
    lines.push("                let response_body = if let Ok(req) = serde_json::from_str::<InvokeRequest>(body_str) {".into());
    lines.push("                    serde_json::to_string(&agent.process_task(&req).await).unwrap_or_default()".into());
    lines.push("                } else {".into());

    if x402 {
        lines.push("                    // x402: check payment header before responding".into());
        lines.push("                    // TODO: validate X-Payment header from request".into());
    }

    if stream {
        lines.push(
            "                    // Streaming: for /invoke/stream, chunk the response".into(),
        );
        lines.push("                    // TODO: send chunked Transfer-Encoding response".into());
    }

    lines.push("                    r#\"{\"status\":\"error\",\"errorMessage\":\"invalid request\"}\"#.to_string()".into());
    lines.push("                };".into());
    lines.push("                let http_resp = format!(\"HTTP/1.1 200 OK\\r\\nContent-Type: application/json\\r\\nContent-Length: {}\\r\\n\\r\\n{}\", response_body.len(), response_body);".into());
    lines.push("                let _ = socket.write_all(http_resp.as_bytes()).await;".into());
    lines.push("            });".into());
    lines.push("        }".into());
    lines.push("    }".into());
    lines.push("}".into());
    lines.push(String::new());

    // uuid helper
    lines.push("fn uuid_v4() -> String {".into());
    lines.push("    use std::time::{SystemTime, UNIX_EPOCH};".into());
    lines.push("    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();".into());
    lines.push("    format!(\"{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}\", t, t >> 16, t & 0xfff, (t >> 8) & 0x3fff | 0x8000, t as u64 * 0x1000193)".into());
    lines.push("}".into());
    lines.push(String::new());

    lines.join("\n")
}

fn gen_rust_plugin_mod(plugins: &[String]) -> String {
    let mods: Vec<String> = plugins.iter().map(|p| format!("pub mod {};", p)).collect();
    mods.join("\n") + "\n"
}

fn gen_rust_plugin_stub(plugin: &str) -> String {
    format!(
        r#"//! {plugin} plugin stub — implement the trait below.
//! See templates/rust/src/plugins/ for full implementations.

use serde_json::{{json, Value}};

pub struct {plugin_upper}Plugin;

impl {plugin_upper}Plugin {{
    pub fn new() -> Self {{ Self }}

    pub async fn invoke(&self, payload: &Value) -> Value {{
        // TODO: implement {plugin} invocation
        json!{{ {{ "status": "success", "result": {{ "message": "stub from {plugin}" }} }} }}
    }}
}}
"#,
        plugin = plugin,
        plugin_upper = pascal_case(plugin),
    )
}

fn gen_rust_env_example(name: &str, discovery: &Discovery) -> String {
    let disc_url = if *discovery == Discovery::Libp2p {
        "# SENTRIX_DISCOVERY_URL=  # not used in libp2p mode"
    } else {
        "SENTRIX_DISCOVERY_URL=http://localhost:8080"
    };
    format!(
        r#"# {name} — environment variables
# Copy to .env and fill in your values.

{disc_url}
SENTRIX_DISCOVERY_KEY=your-api-key-here
SENTRIX_HOST=localhost
PORT=6174
"#,
        name = name,
        disc_url = disc_url,
    )
}

fn gen_rust_readme(name: &str) -> String {
    let lib = name.replace('-', "_");
    format!(
        r#"# {name}

A Sentrix P2P-discoverable agent written in Rust, scaffolded with `sentrix scaffold`.

## Prerequisites

- Rust 1.75+
- cargo

## Quick start

```bash
cp .env.example .env
# Edit .env and set SENTRIX_DISCOVERY_URL, SENTRIX_DISCOVERY_KEY, etc.

cargo run
```

## Invoke the agent

```bash
curl -s -X POST http://localhost:6174/invoke \
  -H 'Content-Type: application/json' \
  -d '{{"capability":"echo","payload":{{"hello":"world"}},"requestId":"req-1","from":"client"}}' | jq .
```

## Build for release

```bash
cargo build --release
./target/release/{lib}
```
"#,
        name = name,
        lib = lib,
    )
}

// ── Zig generators ────────────────────────────────────────────────────────────

fn gen_zig_build(name: &str) -> String {
    let lib = name.replace('-', "_");
    format!(
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target   = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const exe = b.addExecutable(.{{
        .name         = "{lib}",
        .root_source_file = b.path("src/main.zig"),
        .target       = target,
        .optimize     = optimize,
    }});

    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);

    const run_step = b.step("run", "Run the agent");
    run_step.dependOn(&run_cmd.step);

    const unit_tests = b.addTest(.{{
        .root_source_file = b.path("src/main.zig"),
        .target           = target,
        .optimize         = optimize,
    }});
    const run_unit_tests = b.addRunArtifact(unit_tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_unit_tests.step);
}}
"#,
        lib = lib,
    )
}

fn gen_zig_main(agent_struct: &str) -> String {
    format!(
        r#"const std = @import("std");
const agent = @import("agent.zig");

pub fn main() !void {{
    var gpa = std.heap.GeneralPurposeAllocator(.{{}}){{}};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const port_str = std.posix.getenv("PORT") orelse "6174";
    const port = try std.fmt.parseInt(u16, port_str, 10);

    var a = agent.{agent_struct}.init(allocator);
    defer a.deinit();
    try a.start(port);
}}
"#,
        agent_struct = agent_struct,
    )
}

fn gen_zig_agent(
    name: &str,
    agent_struct: &str,
    plugins: &[String],
    discovery: &Discovery,
    did: bool,
    stream: bool,
    x402: bool,
) -> String {
    let disc_type = if *discovery == Discovery::Libp2p {
        "libp2p"
    } else {
        "http"
    };
    let mut lines: Vec<String> = Vec::new();

    lines.push("const std = @import(\"std\");".into());
    lines.push(String::new());

    if !plugins.is_empty() {
        lines.push("const plugins = @import(\"plugins/mod.zig\");".into());
        lines.push(String::new());
    }
    if did {
        lines.push("// DID example: generate an ed25519 key pair".into());
        lines.push("// const ed25519 = std.crypto.sign.Ed25519;".into());
        lines.push(String::new());
    }

    lines.push(format!(
        "pub const {agent_struct} = struct {{",
        agent_struct = agent_struct
    ));
    lines.push("    allocator: std.mem.Allocator,".into());
    lines.push("    agent_id:  [64]u8 = undefined,".into());
    lines.push(String::new());

    lines.push(format!(
        "    pub fn init(allocator: std.mem.Allocator) {agent_struct} {{",
        agent_struct = agent_struct
    ));
    lines.push(format!(
        "        var self = {agent_struct}{{ .allocator = allocator }};",
        agent_struct = agent_struct
    ));
    lines.push(format!(
        "        _ = std.fmt.bufPrint(&self.agent_id, \"sentrix://agent/{name}/{{d}}\", .{{std.time.milliTimestamp()}}) catch {{}};",
        name = name
    ));
    lines.push("        return self;".into());
    lines.push("    }".into());
    lines.push(String::new());

    lines.push("    pub fn deinit(_: *@This()) void {}".into());
    lines.push(String::new());

    lines.push("    pub fn processTask(self: *@This(), capability: []const u8, _payload: []const u8, out: *std.ArrayList(u8)) !void {".into());
    lines.push("        _ = self;".into());
    if plugins.is_empty() {
        lines.push("        if (std.mem.eql(u8, capability, \"echo\")) {".into());
        lines.push("            try out.appendSlice(\"{\\\"status\\\":\\\"success\\\",\\\"result\\\":{\\\"echo\\\":true}}\");".into());
        lines.push("        } else if (std.mem.eql(u8, capability, \"ping\")) {".into());
        lines.push("            try out.appendSlice(\"{\\\"status\\\":\\\"success\\\",\\\"result\\\":{\\\"pong\\\":true}}\");".into());
        lines.push("        } else {".into());
        lines.push("            try out.appendSlice(\"{\\\"status\\\":\\\"error\\\",\\\"errorMessage\\\":\\\"Unknown capability\\\"}\");".into());
        lines.push("        }".into());
    } else {
        lines.push("        // TODO: delegate to the appropriate plugin".into());
        lines.push("        _ = capability;".into());
        lines.push("        _ = _payload;".into());
        lines.push("        try out.appendSlice(\"{\\\"status\\\":\\\"success\\\",\\\"result\\\":{\\\"message\\\":\\\"stub\\\"}}\");".into());
    }
    lines.push("    }".into());
    lines.push(String::new());

    // registerDiscovery
    lines.push("    pub fn registerDiscovery(self: *@This()) void {".into());
    lines.push("        _ = self;".into());
    lines.push(format!(
        "        const disc_type: []const u8 = \"{disc_type}\";",
        disc_type = disc_type
    ));
    lines.push("        _ = disc_type;".into());
    lines.push("        const url = std.posix.getenv(\"SENTRIX_DISCOVERY_URL\") orelse {".into());
    lines.push(
        "            std.debug.print(\"[discovery] SENTRIX_DISCOVERY_URL not set\\n\", .{});"
            .into(),
    );
    lines.push("            return;".into());
    lines.push("        };".into());
    lines.push("        _ = url;".into());
    lines.push("        // TODO: POST registration payload to url/agents".into());
    lines.push("    }".into());
    lines.push(String::new());

    if x402 {
        lines.push("    // x402 micropayment validation stub".into());
        lines.push("    pub fn validatePayment(_: *@This(), _header: []const u8) bool {".into());
        lines.push("        // TODO: validate X-Payment header".into());
        lines.push("        return true;".into());
        lines.push("    }".into());
        lines.push(String::new());
    }

    if stream {
        lines.push("    // SSE streaming stub — write 'data: ...' lines to writer".into());
        lines.push("    pub fn streamTask(self: *@This(), capability: []const u8, payload: []const u8, writer: anytype) !void {".into());
        lines.push("        var buf = std.ArrayList(u8).init(self.allocator);".into());
        lines.push("        defer buf.deinit();".into());
        lines.push("        try self.processTask(capability, payload, &buf);".into());
        lines.push("        try writer.print(\"data: {s}\\n\\n\", .{buf.items});".into());
        lines.push("    }".into());
        lines.push(String::new());
    }

    // start
    lines.push("    pub fn start(self: *@This(), port: u16) !void {".into());
    lines.push("        self.registerDiscovery();".into());
    lines.push(String::new());
    lines.push("        const addr = try std.net.Address.resolveIp(\"0.0.0.0\", port);".into());
    lines.push("        var server = try addr.listen(.{ .reuse_address = true });".into());
    lines.push("        defer server.deinit();".into());
    lines.push(format!(
        "        std.debug.print(\"[{name}] listening on http://localhost:{{d}}\\n\", .{{port}});",
        name = name
    ));
    lines.push(String::new());
    lines.push("        while (true) {".into());
    lines.push("            const conn = try server.accept();".into());
    lines.push("            defer conn.stream.close();".into());
    lines.push(String::new());
    lines.push("            var buf: [8192]u8 = undefined;".into());
    lines.push("            const n = conn.stream.read(&buf) catch continue;".into());
    lines.push("            if (n == 0) continue;".into());
    lines.push(String::new());
    lines.push("            // Naive: extract JSON body after \\r\\n\\r\\n".into());
    lines.push("            const raw = buf[0..n];".into());
    lines.push("            var out = std.ArrayList(u8).init(self.allocator);".into());
    lines.push("            defer out.deinit();".into());
    lines.push(String::new());
    lines.push("            if (std.mem.indexOf(u8, raw, \"\\r\\n\\r\\n\")) |hdr_end| {".into());
    lines.push("                const body = raw[hdr_end + 4..];".into());
    lines.push("                // Extract capability from body (minimal JSON parse)".into());
    lines.push(
        "                const cap = extractJsonField(body, \"capability\") orelse \"echo\";"
            .into(),
    );
    lines.push("                try self.processTask(cap, body, &out);".into());
    lines.push("            } else {".into());
    lines.push("                try out.appendSlice(\"{\\\"status\\\":\\\"error\\\",\\\"errorMessage\\\":\\\"bad request\\\"}\");".into());
    lines.push("            }".into());
    lines.push(String::new());
    lines.push("            const resp_body = out.items;".into());
    lines.push("            var resp_buf: [512]u8 = undefined;".into());
    lines.push("            const header = try std.fmt.bufPrint(&resp_buf,".into());
    lines.push("                \"HTTP/1.1 200 OK\\r\\nContent-Type: application/json\\r\\nContent-Length: {d}\\r\\n\\r\\n\",".into());
    lines.push("                .{resp_body.len});".into());
    lines.push("            _ = try conn.stream.write(header);".into());
    lines.push("            _ = try conn.stream.write(resp_body);".into());
    lines.push("        }".into());
    lines.push("    }".into());
    lines.push("};".into());
    lines.push(String::new());

    // helper
    lines.push("/// Extract a JSON string field value (minimal, no allocations).".into());
    lines.push("fn extractJsonField(json: []const u8, key: []const u8) ?[]const u8 {".into());
    lines.push("    const needle = key;".into());
    lines.push("    const idx = std.mem.indexOf(u8, json, needle) orelse return null;".into());
    lines.push("    const after_key = json[idx + needle.len..];".into());
    lines
        .push("    const colon = std.mem.indexOf(u8, after_key, \":\") orelse return null;".into());
    lines.push("    const val_start_raw = after_key[colon + 1..];".into());
    lines.push("    var val_start = std.mem.trimLeft(u8, val_start_raw, \" \\t\");".into());
    lines.push("    if (val_start.len == 0 or val_start[0] != '\"') return null;".into());
    lines.push("    val_start = val_start[1..];".into());
    lines.push(
        "    const end = std.mem.indexOf(u8, val_start, \"\\\"\") orelse return null;".into(),
    );
    lines.push("    return val_start[0..end];".into());
    lines.push("}".into());
    lines.push(String::new());

    lines.join("\n")
}

fn gen_zig_plugin_mod(plugins: &[String]) -> String {
    let pubs: Vec<String> = plugins
        .iter()
        .map(|p| format!("pub const {} = @import(\"{}.zig\");", pascal_case(p), p))
        .collect();
    pubs.join("\n") + "\n"
}

fn gen_zig_plugin_stub(plugin: &str) -> String {
    let class = pascal_case(plugin);
    format!(
        r#"//! {class} plugin stub.
//! See templates/zig/src/plugins/ for reference implementations.

const std = @import("std");

pub const {class}Plugin = struct {{
    pub fn init() {class}Plugin {{ return .{{}}; }}

    pub fn invoke(_: *const {class}Plugin, _payload: []const u8, out: *std.ArrayList(u8)) !void {{
        // TODO: implement {plugin} invocation
        _ = _payload;
        try out.appendSlice("{{\\"status\\":\\"success\\",\\"result\\":{{\\"message\\":\\"stub from {plugin}\\"}}}}");\
    }}
}};
"#,
        class = class,
        plugin = plugin,
    )
}

fn gen_zig_env_example(name: &str, discovery: &Discovery) -> String {
    let disc_url = if *discovery == Discovery::Libp2p {
        "# SENTRIX_DISCOVERY_URL=  # not used in libp2p mode"
    } else {
        "SENTRIX_DISCOVERY_URL=http://localhost:8080"
    };
    format!(
        r#"# {name} — environment variables
# Copy to .env and fill in your values.

{disc_url}
SENTRIX_DISCOVERY_KEY=your-api-key-here
SENTRIX_HOST=localhost
PORT=6174
"#,
        name = name,
        disc_url = disc_url,
    )
}

fn gen_zig_readme(name: &str) -> String {
    format!(
        r#"# {name}

A Sentrix P2P-discoverable agent written in Zig, scaffolded with `sentrix scaffold`.

## Prerequisites

- Zig 0.13+

## Quick start

```bash
cp .env.example .env
# Edit .env and set SENTRIX_DISCOVERY_URL, SENTRIX_DISCOVERY_KEY, etc.

zig build run
```

## Invoke the agent

```bash
curl -s -X POST http://localhost:6174/invoke \
  -H 'Content-Type: application/json' \
  -d '{{"capability":"echo","payload":{{"hello":"world"}},"requestId":"req-1","from":"client"}}' | jq .
```

## Build for release

```bash
zig build -Doptimize=ReleaseFast
./zig-out/bin/{name}
```
"#,
        name = name,
    )
}

// ── File list assembly ─────────────────────────────────────────────────────────

fn collect_files(
    name: &str,
    lang: &Lang,
    plugins: &[String],
    discovery: &Discovery,
    did: bool,
    stream: bool,
    x402: bool,
) -> Vec<GenFile> {
    let agent_class = pascal_case(name);
    let mut files: Vec<GenFile> = Vec::new();

    match lang {
        Lang::TypeScript => {
            files.push(GenFile::new("package.json", gen_ts_package_json(name)));
            files.push(GenFile::new("tsconfig.json", gen_ts_tsconfig()));
            files.push(GenFile::new("src/index.ts", gen_ts_index(&agent_class)));
            files.push(GenFile::new(
                "src/agent.ts",
                gen_ts_agent(name, &agent_class, plugins, discovery, did, stream, x402),
            ));
            files.push(GenFile::new(
                ".env.example",
                gen_ts_env_example(name, discovery),
            ));
            files.push(GenFile::new(
                "README.md",
                gen_ts_readme(name, discovery, did, stream, x402),
            ));
            for plugin in plugins {
                files.push(GenFile::new(
                    format!("src/plugins/{}", plugin_file_name(plugin)),
                    gen_ts_plugin_stub(plugin),
                ));
            }
        }
        Lang::Rust => {
            files.push(GenFile::new("Cargo.toml", gen_rust_cargo_toml(name)));
            files.push(GenFile::new("src/main.rs", gen_rust_main(&agent_class)));
            files.push(GenFile::new(
                "src/agent.rs",
                gen_rust_agent(name, &agent_class, plugins, discovery, did, stream, x402),
            ));
            files.push(GenFile::new(
                ".env.example",
                gen_rust_env_example(name, discovery),
            ));
            files.push(GenFile::new("README.md", gen_rust_readme(name)));
            if !plugins.is_empty() {
                files.push(GenFile::new(
                    "src/plugins/mod.rs",
                    gen_rust_plugin_mod(plugins),
                ));
                for plugin in plugins {
                    files.push(GenFile::new(
                        format!("src/plugins/{}.rs", plugin),
                        gen_rust_plugin_stub(plugin),
                    ));
                }
            }
        }
        Lang::Zig => {
            files.push(GenFile::new("build.zig", gen_zig_build(name)));
            files.push(GenFile::new("src/main.zig", gen_zig_main(&agent_class)));
            files.push(GenFile::new(
                "src/agent.zig",
                gen_zig_agent(name, &agent_class, plugins, discovery, did, stream, x402),
            ));
            files.push(GenFile::new(
                ".env.example",
                gen_zig_env_example(name, discovery),
            ));
            files.push(GenFile::new("README.md", gen_zig_readme(name)));
            if !plugins.is_empty() {
                files.push(GenFile::new(
                    "src/plugins/mod.zig",
                    gen_zig_plugin_mod(plugins),
                ));
                for plugin in plugins {
                    files.push(GenFile::new(
                        format!("src/plugins/{}.zig", plugin),
                        gen_zig_plugin_stub(plugin),
                    ));
                }
            }
        }
    }

    files
}

// ── Tree printer ──────────────────────────────────────────────────────────────

/// Print a tree view of generated files, grouped by directory.
fn print_tree(name: &str, files: &[GenFile]) {
    logger::title(&format!("{}/", name));

    // Collect unique directory segments for a pretty tree
    let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
    let n = paths.len();
    for (i, path) in paths.iter().enumerate() {
        let is_last = i == n - 1;
        let sym = if is_last { "└──" } else { "├──" };
        logger::tree(sym, path);
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(args: ScaffoldArgs) -> Result<()> {
    // ── Validate ──────────────────────────────────────────────────────────────
    if args.name.is_empty() {
        return Err(anyhow!("Project name must not be empty."));
    }
    if args.name.contains('/') || args.name.contains('\\') {
        return Err(anyhow!(
            "Project name '{}' must not contain path separators.",
            args.name
        ));
    }

    let lang = Lang::from_str(&args.lang).ok_or_else(|| {
        anyhow!(
            "Unknown language '{}'. Valid options: typescript, rust, zig",
            args.lang
        )
    })?;

    let discovery = Discovery::from_str(&args.discovery).ok_or_else(|| {
        anyhow!(
            "Unknown discovery backend '{}'. Valid options: http, libp2p",
            args.discovery
        )
    })?;

    let plugins = parse_plugins(&args.plugins);

    // ── Output directory ──────────────────────────────────────────────────────
    let base_dir = match &args.output {
        Some(d) => d.clone(),
        None => std::env::current_dir().context("Cannot determine current directory")?,
    };
    let project_dir = base_dir.join(&args.name);

    // ── Collect file list ─────────────────────────────────────────────────────
    let files = collect_files(
        &args.name,
        &lang,
        &plugins,
        &discovery,
        args.did,
        args.stream,
        args.x402,
    );

    // ── Dry-run: just print and exit ──────────────────────────────────────────
    if args.dry_run {
        println!(
            "\n{} {}",
            "[dry-run]".bright_black(),
            format!("Would create {} files in {}/", files.len(), args.name).bold()
        );
        print_tree(&args.name, &files);
        println!(
            "\n{}",
            "  (no files written — remove --dry-run to scaffold for real)".dimmed()
        );
        return Ok(());
    }

    // ── Guard: target must not be a non-empty existing directory ──────────────
    if project_dir.exists() {
        let is_empty = project_dir
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(anyhow!(
                "Directory '{}' already exists and is non-empty.",
                project_dir.display()
            ));
        }
    }

    // ── Write files ───────────────────────────────────────────────────────────
    let mut written: Vec<String> = Vec::new();

    for gf in &files {
        let dest = project_dir.join(&gf.rel_path);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
        }

        let mut file = std::fs::File::create(&dest)
            .with_context(|| format!("Failed to create '{}'", dest.display()))?;
        file.write_all(gf.content.as_bytes())
            .with_context(|| format!("Failed to write '{}'", dest.display()))?;

        written.push(gf.rel_path.clone());
    }

    // ── Success summary ───────────────────────────────────────────────────────
    logger::success(&format!(
        "Scaffolded '{}' ({} files, {})",
        args.name,
        written.len(),
        lang.as_str()
    ));

    print_tree(&args.name, &files);

    logger::title("Next steps:");
    match lang {
        Lang::TypeScript => {
            logger::dim(&format!(
                "  cd {name}\n  cp .env.example .env && $EDITOR .env\n  npm install\n  npm run dev",
                name = args.name
            ));
        }
        Lang::Rust => {
            logger::dim(&format!(
                "  cd {name}\n  cp .env.example .env && $EDITOR .env\n  cargo run",
                name = args.name
            ));
        }
        Lang::Zig => {
            logger::dim(&format!(
                "  cd {name}\n  cp .env.example .env && $EDITOR .env\n  zig build run",
                name = args.name
            ));
        }
    }

    logger::dim(
        "\n  Invoke:  curl -s -X POST http://localhost:6174/invoke \\\n             -H 'Content-Type: application/json' \\\n             -d '{\"capability\":\"echo\",\"payload\":{\"hello\":\"world\"},\"requestId\":\"req-1\",\"from\":\"client\"}' | jq .",
    );

    Ok(())
}
